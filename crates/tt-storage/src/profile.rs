//! Profile card + activity blend projections.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use sqlx::PgPool;
use tt_core::domain::{
    ActivityBlend, BlendEvidence, ClientKind, ContributionEvent, ContributionKind, DensitySummary,
    LangWeight, Profile, ProfileCard, RepoContribution, ToolWeight,
};
use tt_core::ids::PrincipalId;

use crate::error::{Result, StorageError};
use crate::events::ContributionStore;

#[async_trait]
pub trait ProfileStore: Send + Sync {
    async fn get(&self, principal_id: PrincipalId) -> Result<Option<Profile>>;
    async fn upsert(&self, profile: &Profile) -> Result<()>;
}

pub struct PostgresProfileStore {
    pool: PgPool,
}

impl PostgresProfileStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProfileStore for PostgresProfileStore {
    async fn get(&self, principal_id: PrincipalId) -> Result<Option<Profile>> {
        let row: Option<(
            Option<String>,
            Option<String>,
            Option<String>,
            serde_json::Value,
            bool,
            chrono::DateTime<Utc>,
        )> = sqlx::query_as(
            r#"
            SELECT bio, location, website, links, is_public, updated_at
            FROM profiles WHERE principal_id = $1
            "#,
        )
        .bind(principal_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;

        Ok(row.map(|(bio, location, website, links, is_public, updated_at)| Profile {
            principal_id,
            bio,
            location,
            website,
            links,
            is_public,
            updated_at,
        }))
    }

    async fn upsert(&self, profile: &Profile) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO profiles (principal_id, bio, location, website, links, is_public, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT (principal_id) DO UPDATE SET
                bio = EXCLUDED.bio,
                location = EXCLUDED.location,
                website = EXCLUDED.website,
                links = EXCLUDED.links,
                is_public = EXCLUDED.is_public,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(profile.principal_id.as_uuid())
        .bind(&profile.bio)
        .bind(&profile.location)
        .bind(&profile.website)
        .bind(&profile.links)
        .bind(profile.is_public)
        .bind(profile.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;
        Ok(())
    }
}

pub struct ProfileService {
    profiles: Arc<dyn ProfileStore>,
    events: Arc<dyn ContributionStore>,
    pool: PgPool,
}

impl ProfileService {
    pub fn new(
        profiles: Arc<dyn ProfileStore>,
        events: Arc<dyn ContributionStore>,
        pool: PgPool,
    ) -> Self {
        Self {
            profiles,
            events,
            pool,
        }
    }

    pub async fn get_or_default(&self, principal_id: PrincipalId) -> Result<Profile> {
        if let Some(p) = self.profiles.get(principal_id).await? {
            return Ok(p);
        }
        Ok(Profile {
            principal_id,
            bio: None,
            location: None,
            website: None,
            links: serde_json::json!([]),
            is_public: true,
            updated_at: Utc::now(),
        })
    }

    pub async fn update(
        &self,
        principal_id: PrincipalId,
        bio: Option<Option<String>>,
        location: Option<Option<String>>,
        website: Option<Option<String>>,
        links: Option<serde_json::Value>,
        is_public: Option<bool>,
    ) -> Result<Profile> {
        let mut p = self.get_or_default(principal_id).await?;
        if let Some(b) = bio {
            p.bio = b;
        }
        if let Some(l) = location {
            p.location = l;
        }
        if let Some(w) = website {
            p.website = w;
        }
        if let Some(links) = links {
            p.links = links;
        }
        if let Some(pub_) = is_public {
            p.is_public = pub_;
        }
        p.updated_at = Utc::now();
        self.profiles.upsert(&p).await?;
        Ok(p)
    }

    pub async fn card(&self, principal_id: PrincipalId, window_days: u32) -> Result<ProfileCard> {
        let window_days = window_days.clamp(7, 365);
        let since = Utc::now() - Duration::days(window_days as i64);

        let (handle, display_name, kind, member_since): (String, String, String, chrono::DateTime<Utc>) =
            sqlx::query_as(
                r#"
                SELECT handle, display_name, kind, created_at
                FROM principals WHERE id = $1
                "#,
            )
            .bind(principal_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StorageError::Other(e.into()))?
            .ok_or(StorageError::NotFound)?;

        let profile = self.get_or_default(principal_id).await?;
        let events = self
            .events
            .list_for_principal(principal_id, since, 5000)
            .await?;

        let blend = compute_blend(&events, window_days);
        let top_languages = compute_languages(&events);
        let top_tools = compute_tools(&events);
        let density = compute_density(&events);

        Ok(ProfileCard {
            principal_id,
            handle,
            display_name,
            kind,
            bio: profile.bio,
            is_public: profile.is_public,
            member_since,
            blend,
            top_languages,
            top_tools,
            density,
        })
    }

    pub async fn repos(
        &self,
        principal_id: PrincipalId,
        window_days: u32,
    ) -> Result<Vec<RepoContribution>> {
        let since = Utc::now() - Duration::days(window_days.clamp(7, 365) as i64);
        let events = self
            .events
            .list_for_principal(principal_id, since, 5000)
            .await?;
        let mut map: HashMap<_, (u64, chrono::DateTime<Utc>, Option<_>)> = HashMap::new();
        for e in events {
            if let Some(rid) = e.repository_id {
                let entry = map.entry(rid).or_insert((0, e.ts, e.workspace_id));
                entry.0 += 1;
                if e.ts > entry.1 {
                    entry.1 = e.ts;
                }
            }
        }
        let mut out: Vec<_> = map
            .into_iter()
            .map(|(repository_id, (events, last_seen, workspace_id))| RepoContribution {
                repository_id,
                workspace_id,
                events,
                last_seen,
            })
            .collect();
        out.sort_by(|a, b| b.events.cmp(&a.events));
        Ok(out)
    }

    pub fn events_store(&self) -> Arc<dyn ContributionStore> {
        self.events.clone()
    }
}

fn compute_blend(events: &[ContributionEvent], window_days: u32) -> ActivityBlend {
    let total = events.len() as u64;
    let mut human = 0u64;
    let mut agent = 0u64;
    let mut ci = 0u64;
    let mut thin = 0u64;

    for e in events {
        match e.client_kind {
            ClientKind::Agent => agent += 1,
            ClientKind::Ci => ci += 1,
            ClientKind::GitHttp | ClientKind::DioxusWeb | ClientKind::DioxusDesktop
            | ClientKind::DioxusMobile | ClientKind::Api => human += 1,
            ClientKind::Unknown => {
                if e.is_automation {
                    ci += 1;
                } else {
                    human += 1;
                }
            }
        }
        if matches!(e.kind, ContributionKind::Commit | ContributionKind::Push) {
            let changed = e.lines_added + e.lines_deleted;
            if e.files_touched <= 1 && changed <= 5 {
                thin += 1;
            }
        }
    }

    let denom = total.max(1) as f32;
    let thin_commits_total = events
        .iter()
        .filter(|e| matches!(e.kind, ContributionKind::Commit | ContributionKind::Push))
        .count() as u64;
    let thin_activity = thin_commits_total > 5 && (thin as f32 / thin_commits_total.max(1) as f32) > 0.6;

    ActivityBlend {
        window_days,
        human_interactive_pct: (human as f32 / denom) * 100.0,
        assisted_agent_pct: (agent as f32 / denom) * 100.0,
        scripted_ci_pct: (ci as f32 / denom) * 100.0,
        thin_activity,
        evidence: BlendEvidence {
            total_events: total,
            human_events: human,
            agent_events: agent,
            ci_events: ci,
            thin_commits: thin,
        },
    }
}

fn compute_languages(events: &[ContributionEvent]) -> Vec<LangWeight> {
    let mut weights: HashMap<String, (u64, u64)> = HashMap::new(); // lang -> (events, line weight)
    for e in events {
        let w = (e.lines_added + e.lines_deleted).max(1) as u64;
        for lang in &e.languages {
            let entry = weights.entry(lang.clone()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += w;
        }
    }
    let total_w: u64 = weights.values().map(|(_, w)| *w).sum::<u64>().max(1);
    let mut out: Vec<_> = weights
        .into_iter()
        .map(|(language, (events, w))| LangWeight {
            language,
            weight_pct: (w as f32 / total_w as f32) * 100.0,
            events,
        })
        .collect();
    out.sort_by(|a, b| {
        b.weight_pct
            .partial_cmp(&a.weight_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(8);
    out
}

fn compute_tools(events: &[ContributionEvent]) -> Vec<ToolWeight> {
    let mut map: HashMap<(String, Option<String>), u64> = HashMap::new();
    for e in events {
        *map.entry((e.client_kind.as_str().to_string(), e.tool_id.clone()))
            .or_insert(0) += 1;
    }
    let total = map.values().sum::<u64>().max(1);
    let mut out: Vec<_> = map
        .into_iter()
        .map(|((client_kind, tool_id), events)| ToolWeight {
            client_kind,
            tool_id,
            weight_pct: (events as f32 / total as f32) * 100.0,
            events,
        })
        .collect();
    out.sort_by(|a, b| b.events.cmp(&a.events));
    out.truncate(8);
    out
}

fn compute_density(events: &[ContributionEvent]) -> DensitySummary {
    let commits: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.kind, ContributionKind::Commit | ContributionKind::Push | ContributionKind::PrMerge))
        .collect();
    let n = commits.len() as f32;
    if n == 0.0 {
        return DensitySummary {
            avg_files_per_commit: 0.0,
            avg_lines_changed_per_commit: 0.0,
            commit_events: 0,
        };
    }
    let files: u32 = commits.iter().map(|e| e.files_touched).sum();
    let lines: u32 = commits
        .iter()
        .map(|e| e.lines_added + e.lines_deleted)
        .sum();
    DensitySummary {
        avg_files_per_commit: files as f32 / n,
        avg_lines_changed_per_commit: lines as f32 / n,
        commit_events: commits.len() as u64,
    }
}

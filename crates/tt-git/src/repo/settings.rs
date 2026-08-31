//! Per-repository merge settings + protections.

use async_trait::async_trait;
use sqlx::PgPool;
use tt_core::domain::{MergeMethod, RepositorySettings};
use tt_core::ids::RepositoryId;

use crate::error::{GitError, Result};

#[async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get(&self, repository_id: RepositoryId) -> Result<RepositorySettings>;
    async fn upsert(&self, settings: &RepositorySettings) -> Result<()>;
}

#[derive(Clone)]
pub struct PostgresSettingsStore {
    pool: PgPool,
}

impl PostgresSettingsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_methods(arr: Vec<String>) -> Vec<MergeMethod> {
    arr.into_iter()
        .filter_map(|s| MergeMethod::parse(&s))
        .collect()
}

#[async_trait]
impl SettingsStore for PostgresSettingsStore {
    async fn get(&self, repository_id: RepositoryId) -> Result<RepositorySettings> {
        let row: Option<(Vec<String>, String, bool, bool, bool, i32, bool, Vec<String>)> = sqlx::query_as(
            r#"
            SELECT allowed_merge_methods, default_merge_method, delete_branch_on_merge,
                   require_up_to_date, restrict_merge_to_admins, required_approvals,
                   COALESCE(block_secret_push, TRUE),
                   COALESCE(secret_push_allowlist, ARRAY[]::TEXT[])
            FROM repository_settings
            WHERE repository_id = $1
            "#,
        )
        .bind(repository_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;

        Ok(match row {
            Some((allowed, default, delete_branch, up_to_date, admins, req_appr, block, allow)) => RepositorySettings {
                repository_id,
                allowed_merge_methods: parse_methods(allowed),
                default_merge_method: MergeMethod::parse(&default)
                    .unwrap_or(MergeMethod::Merge),
                delete_branch_on_merge: delete_branch,
                require_up_to_date: up_to_date,
                restrict_merge_to_admins: admins,
                required_approvals: req_appr.max(0) as u32,
                block_secret_push: block,
                secret_push_allowlist: allow,
            },
            None => RepositorySettings::defaults(repository_id),
        })
    }

    async fn upsert(&self, settings: &RepositorySettings) -> Result<()> {
        if settings.allowed_merge_methods.is_empty() {
            return Err(GitError::InvalidArgument(
                "allowed_merge_methods must not be empty".into(),
            ));
        }
        if !settings.allows(settings.default_merge_method) {
            return Err(GitError::InvalidArgument(
                "default_merge_method must be in allowed_merge_methods".into(),
            ));
        }
        let allowed: Vec<String> = settings
            .allowed_merge_methods
            .iter()
            .map(|m| m.as_str().to_string())
            .collect();
        sqlx::query(
            r#"
            INSERT INTO repository_settings (
                repository_id, allowed_merge_methods, default_merge_method,
                delete_branch_on_merge, require_up_to_date, restrict_merge_to_admins,
                required_approvals, block_secret_push, secret_push_allowlist
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (repository_id) DO UPDATE SET
                allowed_merge_methods = EXCLUDED.allowed_merge_methods,
                default_merge_method = EXCLUDED.default_merge_method,
                delete_branch_on_merge = EXCLUDED.delete_branch_on_merge,
                require_up_to_date = EXCLUDED.require_up_to_date,
                restrict_merge_to_admins = EXCLUDED.restrict_merge_to_admins,
                required_approvals = EXCLUDED.required_approvals,
                block_secret_push = EXCLUDED.block_secret_push,
                secret_push_allowlist = EXCLUDED.secret_push_allowlist,
                updated_at = NOW()
            "#,
        )
        .bind(settings.repository_id.as_uuid())
        .bind(&allowed)
        .bind(settings.default_merge_method.as_str())
        .bind(settings.delete_branch_on_merge)
        .bind(settings.require_up_to_date)
        .bind(settings.restrict_merge_to_admins)
        .bind(settings.required_approvals as i32)
        .bind(settings.block_secret_push)
        .bind(&settings.secret_push_allowlist)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(())
    }
}

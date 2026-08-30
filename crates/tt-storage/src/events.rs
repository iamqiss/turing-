//! Contribution event persistence (Postgres + optional ClickHouse).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::warn;
use tt_core::domain::{
    ClientKind, ContributionEvent, ContributionKind,
};
use tt_core::ids::{PrincipalId, RepositoryId, WorkspaceId};
use uuid::Uuid;

use crate::clickhouse::ClickHouseClient;
use crate::error::{Result, StorageError};

#[async_trait]
pub trait ContributionStore: Send + Sync {
    async fn record(&self, event: &ContributionEvent) -> Result<()>;
    async fn list_for_principal(
        &self,
        principal_id: PrincipalId,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<ContributionEvent>>;
}

pub struct PostgresContributionStore {
    pool: PgPool,
    ch: Arc<ClickHouseClient>,
}

impl PostgresContributionStore {
    pub fn new(pool: PgPool, ch: Arc<ClickHouseClient>) -> Self {
        Self { pool, ch }
    }
}

#[async_trait]
impl ContributionStore for PostgresContributionStore {
    async fn record(&self, event: &ContributionEvent) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO contribution_events (
                id, ts, principal_id, workspace_id, repository_id,
                kind, client_kind, tool_id, languages,
                files_touched, lines_added, lines_deleted,
                is_automation, metadata
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
            "#,
        )
        .bind(event.id)
        .bind(event.ts)
        .bind(event.principal_id.as_uuid())
        .bind(event.workspace_id.as_ref().map(|id| *id.as_uuid()))
        .bind(event.repository_id.as_ref().map(|id| *id.as_uuid()))
        .bind(event.kind.as_str())
        .bind(event.client_kind.as_str())
        .bind(&event.tool_id)
        .bind(&event.languages)
        .bind(event.files_touched as i32)
        .bind(event.lines_added as i32)
        .bind(event.lines_deleted as i32)
        .bind(event.is_automation)
        .bind(&event.metadata)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;

        // Best-effort ClickHouse mirror
        if self.ch.is_enabled() {
            let row = serde_json::json!({
                "id": event.id.to_string(),
                "ts": event.ts.timestamp_millis(),
                "principal_id": event.principal_id.to_string(),
                "workspace_id": event.workspace_id.map(|id| id.to_string()),
                "repository_id": event.repository_id.map(|id| id.to_string()),
                "kind": event.kind.as_str(),
                "client_kind": event.client_kind.as_str(),
                "tool_id": event.tool_id,
                "languages": event.languages,
                "files_touched": event.files_touched,
                "lines_added": event.lines_added,
                "lines_deleted": event.lines_deleted,
                "is_automation": if event.is_automation { 1 } else { 0 },
                "metadata": event.metadata.to_string(),
            });
            if let Err(e) = self
                .ch
                .insert_json_each_row("contribution_events", &[row])
                .await
            {
                warn!(error = %e, "clickhouse insert failed; postgres retained");
            }
        }
        Ok(())
    }

    async fn list_for_principal(
        &self,
        principal_id: PrincipalId,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<ContributionEvent>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, ts, principal_id, workspace_id, repository_id,
                   kind, client_kind, tool_id, languages,
                   files_touched, lines_added, lines_deleted,
                   is_automation, metadata
            FROM contribution_events
            WHERE principal_id = $1 AND ts >= $2
            ORDER BY ts DESC
            LIMIT $3
            "#,
        )
        .bind(principal_id.as_uuid())
        .bind(since)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;
        Ok(rows.into_iter().filter_map(|r| r.into_event()).collect())
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    ts: DateTime<Utc>,
    principal_id: Uuid,
    workspace_id: Option<Uuid>,
    repository_id: Option<Uuid>,
    kind: String,
    client_kind: String,
    tool_id: Option<String>,
    languages: Vec<String>,
    files_touched: i32,
    lines_added: i32,
    lines_deleted: i32,
    is_automation: bool,
    metadata: serde_json::Value,
}

impl EventRow {
    fn into_event(self) -> Option<ContributionEvent> {
        Some(ContributionEvent {
            id: self.id,
            ts: self.ts,
            principal_id: PrincipalId::from_uuid(self.principal_id),
            workspace_id: self.workspace_id.map(WorkspaceId::from_uuid),
            repository_id: self.repository_id.map(RepositoryId::from_uuid),
            kind: ContributionKind::parse(&self.kind)?,
            client_kind: ClientKind::parse(&self.client_kind).unwrap_or(ClientKind::Unknown),
            tool_id: self.tool_id,
            languages: self.languages,
            files_touched: self.files_touched.max(0) as u32,
            lines_added: self.lines_added.max(0) as u32,
            lines_deleted: self.lines_deleted.max(0) as u32,
            is_automation: self.is_automation,
            metadata: self.metadata,
        })
    }
}

/// Helper to build a minimal event.
pub fn new_event(
    principal_id: PrincipalId,
    kind: ContributionKind,
    client_kind: ClientKind,
) -> ContributionEvent {
    ContributionEvent {
        id: Uuid::now_v7(),
        ts: Utc::now(),
        principal_id,
        workspace_id: None,
        repository_id: None,
        kind,
        client_kind,
        tool_id: None,
        languages: vec![],
        files_touched: 0,
        lines_added: 0,
        lines_deleted: 0,
        is_automation: matches!(client_kind, ClientKind::Agent | ClientKind::Ci),
        metadata: serde_json::json!({}),
    }
}

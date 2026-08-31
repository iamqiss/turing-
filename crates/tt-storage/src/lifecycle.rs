//! Compute lifecycle events — Postgres source of truth + ClickHouse mirror.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::warn;
use tt_core::domain::{LifecycleEvent, LifecycleKind};
use tt_core::ids::{InstanceId, NodeId, OrgId, PrincipalId, WorkloadId};
use uuid::Uuid;

use crate::clickhouse::ClickHouseClient;
use crate::error::{Result, StorageError};

#[async_trait]
pub trait LifecycleStore: Send + Sync {
    async fn record(&self, event: &LifecycleEvent) -> Result<()>;
    async fn list_for_org(
        &self,
        org_id: OrgId,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<LifecycleEvent>>;
    async fn list_platform(
        &self,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<LifecycleEvent>>;
}

pub struct PostgresLifecycleStore {
    pool: PgPool,
    ch: Arc<ClickHouseClient>,
}

impl PostgresLifecycleStore {
    pub fn new(pool: PgPool, ch: Arc<ClickHouseClient>) -> Self {
        Self { pool, ch }
    }
}

pub fn new_lifecycle(
    org_id: Option<OrgId>,
    kind: LifecycleKind,
    principal_id: Option<PrincipalId>,
) -> LifecycleEvent {
    LifecycleEvent {
        id: Uuid::now_v7(),
        ts: Utc::now(),
        org_id,
        principal_id,
        kind,
        workload_id: None,
        instance_id: None,
        node_id: None,
        region: None,
        metadata: serde_json::json!({}),
    }
}

#[async_trait]
impl LifecycleStore for PostgresLifecycleStore {
    async fn record(&self, event: &LifecycleEvent) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO lifecycle_events (
                id, ts, org_id, principal_id, kind,
                workload_id, instance_id, node_id, region, metadata
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            "#,
        )
        .bind(event.id)
        .bind(event.ts)
        .bind(event.org_id.as_ref().map(|id| *id.as_uuid()))
        .bind(event.principal_id.as_ref().map(|id| *id.as_uuid()))
        .bind(event.kind.as_str())
        .bind(event.workload_id.as_ref().map(|id| *id.as_uuid()))
        .bind(event.instance_id.as_ref().map(|id| *id.as_uuid()))
        .bind(event.node_id.as_ref().map(|id| *id.as_uuid()))
        .bind(&event.region)
        .bind(&event.metadata)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;

        if self.ch.is_enabled() {
            let row = serde_json::json!({
                "id": event.id.to_string(),
                "ts": event.ts.timestamp_millis(),
                "org_id": event.org_id.map(|id| id.to_string()),
                "principal_id": event.principal_id.map(|id| id.to_string()),
                "kind": event.kind.as_str(),
                "workload_id": event.workload_id.map(|id| id.to_string()),
                "instance_id": event.instance_id.map(|id| id.to_string()),
                "node_id": event.node_id.map(|id| id.to_string()),
                "region": event.region,
                "metadata": event.metadata.to_string(),
            });
            if let Err(e) = self.ch.insert_json_each_row("lifecycle_events", &[row]).await {
                warn!(error = %e, "clickhouse lifecycle mirror failed");
            }
        }
        Ok(())
    }

    async fn list_for_org(
        &self,
        org_id: OrgId,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<LifecycleEvent>> {
        let rows: Vec<(
            Uuid,
            DateTime<Utc>,
            Option<Uuid>,
            Option<Uuid>,
            String,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<String>,
            serde_json::Value,
        )> = sqlx::query_as(
            r#"
            SELECT id, ts, org_id, principal_id, kind,
                   workload_id, instance_id, node_id, region, metadata
            FROM lifecycle_events
            WHERE org_id = $1 AND ts >= $2
            ORDER BY ts DESC
            LIMIT $3
            "#,
        )
        .bind(org_id.as_uuid())
        .bind(since)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;

        Ok(rows
            .into_iter()
            .filter_map(|(id, ts, org, pid, kind, wl, inst, node, region, metadata)| {
                let kind = LifecycleKind::parse(&kind)?;
                Some(LifecycleEvent {
                    id,
                    ts,
                    org_id: org.map(OrgId::from_uuid),
                    principal_id: pid.map(PrincipalId::from_uuid),
                    kind,
                    workload_id: wl.map(WorkloadId::from_uuid),
                    instance_id: inst.map(InstanceId::from_uuid),
                    node_id: node.map(NodeId::from_uuid),
                    region,
                    metadata,
                })
            })
            .collect())
    }

    async fn list_platform(
        &self,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<LifecycleEvent>> {
        let rows: Vec<(
            Uuid,
            DateTime<Utc>,
            Option<Uuid>,
            Option<Uuid>,
            String,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<String>,
            serde_json::Value,
        )> = sqlx::query_as(
            r#"
            SELECT id, ts, org_id, principal_id, kind,
                   workload_id, instance_id, node_id, region, metadata
            FROM lifecycle_events
            WHERE org_id IS NULL AND ts >= $1
            ORDER BY ts DESC
            LIMIT $2
            "#,
        )
        .bind(since)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(e.into()))?;

        Ok(rows
            .into_iter()
            .filter_map(|(id, ts, org, pid, kind, wl, inst, node, region, metadata)| {
                let kind = LifecycleKind::parse(&kind)?;
                Some(LifecycleEvent {
                    id,
                    ts,
                    org_id: org.map(OrgId::from_uuid),
                    principal_id: pid.map(PrincipalId::from_uuid),
                    kind,
                    workload_id: wl.map(WorkloadId::from_uuid),
                    instance_id: inst.map(InstanceId::from_uuid),
                    node_id: node.map(NodeId::from_uuid),
                    region,
                    metadata,
                })
            })
            .collect())
    }
}

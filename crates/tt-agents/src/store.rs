use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tt_core::domain::{AgentApproval, AgentRun, AgentRunStatus};
use tt_core::ids::{OrgId, PrincipalId, WorkspaceId};
use uuid::Uuid;

use crate::error::{AgentError, Result};

#[async_trait]
pub trait AgentStore: Send + Sync {
    async fn insert_run(&self, run: &AgentRun) -> Result<()>;
    async fn update_run(&self, run: &AgentRun) -> Result<()>;
    async fn get_run(&self, id: Uuid) -> Result<Option<AgentRun>>;
    async fn list_runs_for_owner(&self, owner_id: PrincipalId, limit: u32) -> Result<Vec<AgentRun>>;
    async fn insert_step(
        &self,
        id: Uuid,
        run_id: Uuid,
        step_index: u32,
        role: &str,
        content: &str,
        tool_name: Option<&str>,
        tool_args: Option<serde_json::Value>,
        tool_result: Option<serde_json::Value>,
    ) -> Result<()>;
    async fn insert_approval(&self, a: &AgentApproval) -> Result<()>;
    async fn resolve_approval(
        &self,
        id: Uuid,
        status: &str,
        resolved_by: PrincipalId,
    ) -> Result<Option<AgentApproval>>;
    async fn get_approval(&self, id: Uuid) -> Result<Option<AgentApproval>>;
}

pub struct PostgresAgentStore {
    pool: PgPool,
}

impl PostgresAgentStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentStore for PostgresAgentStore {
    async fn insert_run(&self, run: &AgentRun) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_runs (
                id, agent_id, owner_id, workspace_id, org_id, status, goal, model,
                max_steps, step_count, result_summary, error, metadata,
                created_at, updated_at, finished_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
            "#,
        )
        .bind(run.id)
        .bind(run.agent_id.as_uuid())
        .bind(run.owner_id.as_uuid())
        .bind(run.workspace_id.as_ref().map(|id| *id.as_uuid()))
        .bind(run.org_id.as_ref().map(|id| *id.as_uuid()))
        .bind(run.status.as_str())
        .bind(&run.goal)
        .bind(&run.model)
        .bind(run.max_steps as i32)
        .bind(run.step_count as i32)
        .bind(&run.result_summary)
        .bind(&run.error)
        .bind(&run.metadata)
        .bind(run.created_at)
        .bind(run.updated_at)
        .bind(run.finished_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        Ok(())
    }

    async fn update_run(&self, run: &AgentRun) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE agent_runs SET
                status = $2, step_count = $3, result_summary = $4, error = $5,
                metadata = $6, updated_at = $7, finished_at = $8
            WHERE id = $1
            "#,
        )
        .bind(run.id)
        .bind(run.status.as_str())
        .bind(run.step_count as i32)
        .bind(&run.result_summary)
        .bind(&run.error)
        .bind(&run.metadata)
        .bind(run.updated_at)
        .bind(run.finished_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        Ok(())
    }

    async fn get_run(&self, id: Uuid) -> Result<Option<AgentRun>> {
        let row: Option<(
            Uuid,
            Uuid,
            Uuid,
            Option<Uuid>,
            Option<Uuid>,
            String,
            String,
            String,
            i32,
            i32,
            Option<String>,
            Option<String>,
            serde_json::Value,
            DateTime<Utc>,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            r#"
            SELECT id, agent_id, owner_id, workspace_id, org_id, status, goal, model,
                   max_steps, step_count, result_summary, error, metadata,
                   created_at, updated_at, finished_at
            FROM agent_runs WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        Ok(row.map(
            |(
                id,
                agent_id,
                owner_id,
                ws,
                org,
                status,
                goal,
                model,
                max_steps,
                step_count,
                result_summary,
                error,
                metadata,
                created_at,
                updated_at,
                finished_at,
            )| AgentRun {
                id,
                agent_id: PrincipalId::from_uuid(agent_id),
                owner_id: PrincipalId::from_uuid(owner_id),
                workspace_id: ws.map(WorkspaceId::from_uuid),
                org_id: org.map(OrgId::from_uuid),
                status: AgentRunStatus::parse(&status).unwrap_or(AgentRunStatus::Failed),
                goal,
                model,
                max_steps: max_steps.max(0) as u32,
                step_count: step_count.max(0) as u32,
                result_summary,
                error,
                metadata,
                created_at,
                updated_at,
                finished_at,
            },
        ))
    }

    async fn list_runs_for_owner(&self, owner_id: PrincipalId, limit: u32) -> Result<Vec<AgentRun>> {
        let rows: Vec<(
            Uuid,
            Uuid,
            Uuid,
            Option<Uuid>,
            Option<Uuid>,
            String,
            String,
            String,
            i32,
            i32,
            Option<String>,
            Option<String>,
            serde_json::Value,
            DateTime<Utc>,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            r#"
            SELECT id, agent_id, owner_id, workspace_id, org_id, status, goal, model,
                   max_steps, step_count, result_summary, error, metadata,
                   created_at, updated_at, finished_at
            FROM agent_runs WHERE owner_id = $1
            ORDER BY created_at DESC LIMIT $2
            "#,
        )
        .bind(owner_id.as_uuid())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    agent_id,
                    owner_id,
                    ws,
                    org,
                    status,
                    goal,
                    model,
                    max_steps,
                    step_count,
                    result_summary,
                    error,
                    metadata,
                    created_at,
                    updated_at,
                    finished_at,
                )| AgentRun {
                    id,
                    agent_id: PrincipalId::from_uuid(agent_id),
                    owner_id: PrincipalId::from_uuid(owner_id),
                    workspace_id: ws.map(WorkspaceId::from_uuid),
                    org_id: org.map(OrgId::from_uuid),
                    status: AgentRunStatus::parse(&status).unwrap_or(AgentRunStatus::Failed),
                    goal,
                    model,
                    max_steps: max_steps.max(0) as u32,
                    step_count: step_count.max(0) as u32,
                    result_summary,
                    error,
                    metadata,
                    created_at,
                    updated_at,
                    finished_at,
                },
            )
            .collect())
    }

    async fn insert_step(
        &self,
        id: Uuid,
        run_id: Uuid,
        step_index: u32,
        role: &str,
        content: &str,
        tool_name: Option<&str>,
        tool_args: Option<serde_json::Value>,
        tool_result: Option<serde_json::Value>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_run_steps (
                id, run_id, step_index, role, content, tool_name, tool_args, tool_result, created_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,NOW())
            "#,
        )
        .bind(id)
        .bind(run_id)
        .bind(step_index as i32)
        .bind(role)
        .bind(content)
        .bind(tool_name)
        .bind(tool_args)
        .bind(tool_result)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        Ok(())
    }

    async fn insert_approval(&self, a: &AgentApproval) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_approvals (
                id, run_id, tool_name, tool_args, status, requested_at, resolved_at, resolved_by
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            "#,
        )
        .bind(a.id)
        .bind(a.run_id)
        .bind(&a.tool_name)
        .bind(&a.tool_args)
        .bind(&a.status)
        .bind(a.requested_at)
        .bind(a.resolved_at)
        .bind(a.resolved_by.as_ref().map(|id| *id.as_uuid()))
        .execute(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        Ok(())
    }

    async fn resolve_approval(
        &self,
        id: Uuid,
        status: &str,
        resolved_by: PrincipalId,
    ) -> Result<Option<AgentApproval>> {
        sqlx::query(
            r#"
            UPDATE agent_approvals
            SET status = $2, resolved_at = NOW(), resolved_by = $3
            WHERE id = $1 AND status = 'pending'
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(resolved_by.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        self.get_approval(id).await
    }

    async fn get_approval(&self, id: Uuid) -> Result<Option<AgentApproval>> {
        let row: Option<(
            Uuid,
            Uuid,
            String,
            serde_json::Value,
            String,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            Option<Uuid>,
        )> = sqlx::query_as(
            r#"
            SELECT id, run_id, tool_name, tool_args, status, requested_at, resolved_at, resolved_by
            FROM agent_approvals WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AgentError::Other(e.into()))?;
        Ok(row.map(
            |(id, run_id, tool_name, tool_args, status, requested_at, resolved_at, resolved_by)| {
                AgentApproval {
                    id,
                    run_id,
                    tool_name,
                    tool_args,
                    status,
                    requested_at,
                    resolved_at,
                    resolved_by: resolved_by.map(PrincipalId::from_uuid),
                }
            },
        ))
    }
}

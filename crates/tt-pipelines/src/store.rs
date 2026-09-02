//! Postgres persistence for Turing Tasks.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use tt_core::domain::*;
use tt_core::ids::*;
use uuid::Uuid;

use crate::error::{PipelineError, Result};

#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn upsert_pipeline(&self, p: &TaskPipeline) -> Result<()>;
    async fn list_pipelines(&self, repository_id: RepositoryId) -> Result<Vec<TaskPipeline>>;
    async fn insert_run(&self, r: &TaskRun) -> Result<()>;
    async fn update_run(&self, r: &TaskRun) -> Result<()>;
    async fn get_run(&self, id: Uuid) -> Result<Option<TaskRun>>;
    async fn list_runs(&self, repository_id: RepositoryId, limit: i64) -> Result<Vec<TaskRun>>;
    async fn insert_job(&self, j: &TaskJob) -> Result<()>;
    async fn update_job(&self, j: &TaskJob) -> Result<()>;
    async fn list_jobs(&self, run_id: Uuid) -> Result<Vec<TaskJob>>;
    async fn sum_org_minutes(&self, org_id: OrgId, period_ym: &str) -> Result<u32>;
}

pub struct PostgresTaskStore {
    pool: PgPool,
}

impl PostgresTaskStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_db(e: sqlx::Error) -> PipelineError {
    PipelineError::Other(e.into())
}

#[async_trait]
impl TaskStore for PostgresTaskStore {
    async fn upsert_pipeline(&self, p: &TaskPipeline) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO task_pipelines
              (id, repository_id, org_id, name, path, definition_yaml, enabled, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            ON CONFLICT (repository_id, name) DO UPDATE SET
              path = EXCLUDED.path,
              definition_yaml = EXCLUDED.definition_yaml,
              enabled = EXCLUDED.enabled,
              org_id = EXCLUDED.org_id,
              updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(p.id)
        .bind(p.repository_id.as_uuid())
        .bind(p.org_id.map(|o| *o.as_uuid()))
        .bind(&p.name)
        .bind(&p.path)
        .bind(&p.definition_yaml)
        .bind(p.enabled)
        .bind(p.updated_at)
        .execute(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(())
    }

    async fn list_pipelines(&self, repository_id: RepositoryId) -> Result<Vec<TaskPipeline>> {
        let rows: Vec<(Uuid, Uuid, Option<Uuid>, String, String, String, bool, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                r#"SELECT id, repository_id, org_id, name, path, definition_yaml, enabled, updated_at
                   FROM task_pipelines WHERE repository_id = $1 ORDER BY name"#,
            )
            .bind(repository_id.as_uuid())
            .fetch_all(&self.pool)
            .await
            .map_err(map_db)?;
        Ok(rows
            .into_iter()
            .map(|(id, rid, oid, name, path, yaml, en, at)| TaskPipeline {
                id,
                repository_id: RepositoryId::from_uuid(rid),
                org_id: oid.map(OrgId::from_uuid),
                name,
                path,
                definition_yaml: yaml,
                enabled: en,
                updated_at: at,
            })
            .collect())
    }

    async fn insert_run(&self, r: &TaskRun) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO task_runs (
              id, pipeline_id, repository_id, org_id, pipeline_name, status, trigger,
              git_ref, git_sha, triggered_by, billable_minutes, started_at, finished_at, error, created_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
            "#,
        )
        .bind(r.id)
        .bind(r.pipeline_id)
        .bind(r.repository_id.as_uuid())
        .bind(r.org_id.map(|o| *o.as_uuid()))
        .bind(&r.pipeline_name)
        .bind(r.status.as_str())
        .bind(r.trigger.as_str())
        .bind(&r.git_ref)
        .bind(&r.git_sha)
        .bind(r.triggered_by.map(|p| *p.as_uuid()))
        .bind(r.billable_minutes as i32)
        .bind(r.started_at)
        .bind(r.finished_at)
        .bind(&r.error)
        .bind(r.created_at)
        .execute(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(())
    }

    async fn update_run(&self, r: &TaskRun) -> Result<()> {
        sqlx::query(
            r#"UPDATE task_runs SET status=$2, billable_minutes=$3, started_at=$4, finished_at=$5, error=$6
               WHERE id=$1"#,
        )
        .bind(r.id)
        .bind(r.status.as_str())
        .bind(r.billable_minutes as i32)
        .bind(r.started_at)
        .bind(r.finished_at)
        .bind(&r.error)
        .execute(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(())
    }

    async fn get_run(&self, id: Uuid) -> Result<Option<TaskRun>> {
        let row: Option<(
            Uuid, Uuid, Uuid, Option<Uuid>, String, String, String,
            Option<String>, Option<String>, Option<Uuid>, i32,
            Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>,
            Option<String>, chrono::DateTime<Utc>,
        )> = sqlx::query_as(
            r#"SELECT id, pipeline_id, repository_id, org_id, pipeline_name, status, trigger,
                      git_ref, git_sha, triggered_by, billable_minutes, started_at, finished_at, error, created_at
               FROM task_runs WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(row.map(row_to_run))
    }

    async fn list_runs(&self, repository_id: RepositoryId, limit: i64) -> Result<Vec<TaskRun>> {
        let rows: Vec<(
            Uuid, Uuid, Uuid, Option<Uuid>, String, String, String,
            Option<String>, Option<String>, Option<Uuid>, i32,
            Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>,
            Option<String>, chrono::DateTime<Utc>,
        )> = sqlx::query_as(
            r#"SELECT id, pipeline_id, repository_id, org_id, pipeline_name, status, trigger,
                      git_ref, git_sha, triggered_by, billable_minutes, started_at, finished_at, error, created_at
               FROM task_runs WHERE repository_id = $1 ORDER BY created_at DESC LIMIT $2"#,
        )
        .bind(repository_id.as_uuid())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(rows.into_iter().map(row_to_run).collect())
    }

    async fn insert_job(&self, j: &TaskJob) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO task_jobs
                 (id, run_id, name, status, runs_on, billable_minutes, log_excerpt, started_at, finished_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(j.id)
        .bind(j.run_id)
        .bind(&j.name)
        .bind(j.status.as_str())
        .bind(&j.runs_on)
        .bind(j.billable_minutes as i32)
        .bind(&j.log_excerpt)
        .bind(j.started_at)
        .bind(j.finished_at)
        .execute(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(())
    }

    async fn update_job(&self, j: &TaskJob) -> Result<()> {
        sqlx::query(
            r#"UPDATE task_jobs SET status=$2, billable_minutes=$3, log_excerpt=$4, started_at=$5, finished_at=$6
               WHERE id=$1"#,
        )
        .bind(j.id)
        .bind(j.status.as_str())
        .bind(j.billable_minutes as i32)
        .bind(&j.log_excerpt)
        .bind(j.started_at)
        .bind(j.finished_at)
        .execute(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(())
    }

    async fn list_jobs(&self, run_id: Uuid) -> Result<Vec<TaskJob>> {
        let rows: Vec<(
            Uuid, Uuid, String, String, String, i32,
            Option<String>, Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>,
        )> = sqlx::query_as(
            r#"SELECT id, run_id, name, status, runs_on, billable_minutes, log_excerpt, started_at, finished_at
               FROM task_jobs WHERE run_id = $1 ORDER BY name"#,
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db)?;
        Ok(rows
            .into_iter()
            .map(|(id, run_id, name, status, runs_on, mins, log, st, fin)| TaskJob {
                id,
                run_id,
                name,
                status: TaskRunStatus::parse(&status).unwrap_or(TaskRunStatus::Queued),
                runs_on,
                billable_minutes: mins as u32,
                log_excerpt: log,
                started_at: st,
                finished_at: fin,
            })
            .collect())
    }

    async fn sum_org_minutes(&self, org_id: OrgId, period_ym: &str) -> Result<u32> {
        let row: (i64,) = sqlx::query_as(
            r#"SELECT COALESCE(SUM(billable_minutes),0)::bigint FROM task_runs
               WHERE org_id = $1 AND to_char(created_at, 'YYYY-MM') = $2"#,
        )
        .bind(org_id.as_uuid())
        .bind(period_ym)
        .fetch_one(&self.pool)
        .await
        .unwrap_or((0,));
        Ok(row.0 as u32)
    }
}

fn row_to_run(
    r: (
        Uuid, Uuid, Uuid, Option<Uuid>, String, String, String,
        Option<String>, Option<String>, Option<Uuid>, i32,
        Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>,
        Option<String>, chrono::DateTime<Utc>,
    ),
) -> TaskRun {
    TaskRun {
        id: r.0,
        pipeline_id: r.1,
        repository_id: RepositoryId::from_uuid(r.2),
        org_id: r.3.map(OrgId::from_uuid),
        pipeline_name: r.4,
        status: TaskRunStatus::parse(&r.5).unwrap_or(TaskRunStatus::Queued),
        trigger: TaskTrigger::parse(&r.6).unwrap_or(TaskTrigger::Manual),
        git_ref: r.7,
        git_sha: r.8,
        triggered_by: r.9.map(PrincipalId::from_uuid),
        billable_minutes: r.10 as u32,
        started_at: r.11,
        finished_at: r.12,
        error: r.13,
        created_at: r.14,
    }
}

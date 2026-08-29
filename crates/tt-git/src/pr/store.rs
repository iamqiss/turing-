//! Postgres persistence for pull requests.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tt_core::domain::{PullRequest, PullRequestStatus};
use tt_core::ids::{PrincipalId, PullRequestId, RepositoryId};
use uuid::Uuid;

use crate::error::{GitError, Result};

#[async_trait]
pub trait PullRequestStore: Send + Sync {
    async fn next_number(&self, repository_id: RepositoryId) -> Result<i32>;
    async fn insert(&self, pr: &PullRequest) -> Result<()>;
    async fn get(&self, id: PullRequestId) -> Result<Option<PullRequest>>;
    async fn get_by_number(
        &self,
        repository_id: RepositoryId,
        number: i32,
    ) -> Result<Option<PullRequest>>;
    async fn list(
        &self,
        repository_id: RepositoryId,
        status: Option<PullRequestStatus>,
        limit: u32,
    ) -> Result<Vec<PullRequest>>;
    async fn update_metadata(&self, pr: &PullRequest) -> Result<()>;
    async fn update_status(&self, pr: &PullRequest) -> Result<()>;
}

#[derive(Clone)]
pub struct PostgresPullRequestStore {
    pool: PgPool,
}

impl PostgresPullRequestStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct PrRow {
    id: Uuid,
    repository_id: Uuid,
    number: i32,
    title: String,
    body: Option<String>,
    author_id: Uuid,
    source_branch: String,
    target_branch: String,
    status: String,
    merged_by: Option<Uuid>,
    merged_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<PrRow> for PullRequest {
    fn from(r: PrRow) -> Self {
        PullRequest {
            id: PullRequestId::from_uuid(r.id),
            repository_id: RepositoryId::from_uuid(r.repository_id),
            number: r.number,
            title: r.title,
            body: r.body,
            author_id: PrincipalId::from_uuid(r.author_id),
            source_branch: r.source_branch,
            target_branch: r.target_branch,
            status: PullRequestStatus::parse(&r.status).unwrap_or(PullRequestStatus::Open),
            merged_by: r.merged_by.map(PrincipalId::from_uuid),
            merged_at: r.merged_at,
            closed_at: r.closed_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[async_trait]
impl PullRequestStore for PostgresPullRequestStore {
    async fn next_number(&self, repository_id: RepositoryId) -> Result<i32> {
        let mut tx = self.pool.begin().await.map_err(|e| GitError::Other(e.into()))?;
        sqlx::query(
            r#"
            INSERT INTO repository_pr_counters (repository_id, last_number)
            VALUES ($1, 1)
            ON CONFLICT (repository_id) DO UPDATE
              SET last_number = repository_pr_counters.last_number + 1
            "#,
        )
        .bind(repository_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|e| GitError::Other(e.into()))?;

        let (n,): (i32,) = sqlx::query_as(
            r#"SELECT last_number FROM repository_pr_counters WHERE repository_id = $1"#,
        )
        .bind(repository_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| GitError::Other(e.into()))?;

        tx.commit().await.map_err(|e| GitError::Other(e.into()))?;
        Ok(n)
    }

    async fn insert(&self, pr: &PullRequest) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO pull_requests (
                id, repository_id, number, title, body, author_id,
                source_branch, target_branch, status,
                merged_by, merged_at, closed_at, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
            "#,
        )
        .bind(pr.id.as_uuid())
        .bind(pr.repository_id.as_uuid())
        .bind(pr.number)
        .bind(&pr.title)
        .bind(&pr.body)
        .bind(pr.author_id.as_uuid())
        .bind(&pr.source_branch)
        .bind(&pr.target_branch)
        .bind(pr.status.as_str())
        .bind(pr.merged_by.as_ref().map(|id| *id.as_uuid()))
        .bind(pr.merged_at)
        .bind(pr.closed_at)
        .bind(pr.created_at)
        .bind(pr.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(())
    }

    async fn get(&self, id: PullRequestId) -> Result<Option<PullRequest>> {
        let row = sqlx::query_as::<_, PrRow>(
            r#"
            SELECT id, repository_id, number, title, body, author_id,
                   source_branch, target_branch, status,
                   merged_by, merged_at, closed_at, created_at, updated_at
            FROM pull_requests WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn get_by_number(
        &self,
        repository_id: RepositoryId,
        number: i32,
    ) -> Result<Option<PullRequest>> {
        let row = sqlx::query_as::<_, PrRow>(
            r#"
            SELECT id, repository_id, number, title, body, author_id,
                   source_branch, target_branch, status,
                   merged_by, merged_at, closed_at, created_at, updated_at
            FROM pull_requests
            WHERE repository_id = $1 AND number = $2
            "#,
        )
        .bind(repository_id.as_uuid())
        .bind(number)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list(
        &self,
        repository_id: RepositoryId,
        status: Option<PullRequestStatus>,
        limit: u32,
    ) -> Result<Vec<PullRequest>> {
        let rows = if let Some(st) = status {
            sqlx::query_as::<_, PrRow>(
                r#"
                SELECT id, repository_id, number, title, body, author_id,
                       source_branch, target_branch, status,
                       merged_by, merged_at, closed_at, created_at, updated_at
                FROM pull_requests
                WHERE repository_id = $1 AND status = $2
                ORDER BY number DESC
                LIMIT $3
                "#,
            )
            .bind(repository_id.as_uuid())
            .bind(st.as_str())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, PrRow>(
                r#"
                SELECT id, repository_id, number, title, body, author_id,
                       source_branch, target_branch, status,
                       merged_by, merged_at, closed_at, created_at, updated_at
                FROM pull_requests
                WHERE repository_id = $1
                ORDER BY number DESC
                LIMIT $2
                "#,
            )
            .bind(repository_id.as_uuid())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_metadata(&self, pr: &PullRequest) -> Result<()> {
        let res = sqlx::query(
            r#"
            UPDATE pull_requests
            SET title = $2, body = $3, updated_at = $4
            WHERE id = $1
            "#,
        )
        .bind(pr.id.as_uuid())
        .bind(&pr.title)
        .bind(&pr.body)
        .bind(pr.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        if res.rows_affected() == 0 {
            return Err(GitError::PrNotFound);
        }
        Ok(())
    }

    async fn update_status(&self, pr: &PullRequest) -> Result<()> {
        let res = sqlx::query(
            r#"
            UPDATE pull_requests
            SET status = $2, merged_by = $3, merged_at = $4, closed_at = $5, updated_at = $6
            WHERE id = $1
            "#,
        )
        .bind(pr.id.as_uuid())
        .bind(pr.status.as_str())
        .bind(pr.merged_by.as_ref().map(|id| *id.as_uuid()))
        .bind(pr.merged_at)
        .bind(pr.closed_at)
        .bind(pr.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        if res.rows_affected() == 0 {
            return Err(GitError::PrNotFound);
        }
        Ok(())
    }
}

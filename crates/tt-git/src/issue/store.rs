//! Postgres persistence for issues + comments.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tt_core::domain::{Issue, IssueComment, IssueState};
use tt_core::ids::{IssueCommentId, IssueId, PrincipalId, RepositoryId};
use uuid::Uuid;

use crate::error::{GitError, Result};

#[async_trait]
pub trait IssueStore: Send + Sync {
    async fn next_number(&self, repository_id: RepositoryId) -> Result<i32>;
    async fn insert(&self, issue: &Issue) -> Result<()>;
    async fn get(&self, id: IssueId) -> Result<Option<Issue>>;
    async fn get_by_number(
        &self,
        repository_id: RepositoryId,
        number: i32,
    ) -> Result<Option<Issue>>;
    async fn list(
        &self,
        repository_id: RepositoryId,
        state: Option<IssueState>,
        limit: u32,
    ) -> Result<Vec<Issue>>;
    async fn update(&self, issue: &Issue) -> Result<()>;
    async fn insert_comment(&self, c: &IssueComment) -> Result<()>;
    async fn list_comments(&self, issue_id: IssueId, limit: u32) -> Result<Vec<IssueComment>>;
}

#[derive(Clone)]
pub struct PostgresIssueStore {
    pool: PgPool,
}

impl PostgresIssueStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct IssueRow {
    id: Uuid,
    repository_id: Uuid,
    number: i32,
    title: String,
    body: Option<String>,
    author_id: Uuid,
    state: String,
    closed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<IssueRow> for Issue {
    fn from(r: IssueRow) -> Self {
        Issue {
            id: IssueId::from_uuid(r.id),
            repository_id: RepositoryId::from_uuid(r.repository_id),
            number: r.number,
            title: r.title,
            body: r.body,
            author_id: PrincipalId::from_uuid(r.author_id),
            state: IssueState::parse(&r.state).unwrap_or(IssueState::Open),
            closed_at: r.closed_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(FromRow)]
struct CommentRow {
    id: Uuid,
    issue_id: Uuid,
    author_id: Uuid,
    body: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<CommentRow> for IssueComment {
    fn from(r: CommentRow) -> Self {
        IssueComment {
            id: IssueCommentId::from_uuid(r.id),
            issue_id: IssueId::from_uuid(r.issue_id),
            author_id: PrincipalId::from_uuid(r.author_id),
            body: r.body,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

const ISSUE_COLS: &str = r#"
    id, repository_id, number, title, body, author_id, state, closed_at, created_at, updated_at
"#;

#[async_trait]
impl IssueStore for PostgresIssueStore {
    async fn next_number(&self, repository_id: RepositoryId) -> Result<i32> {
        let mut tx = self.pool.begin().await.map_err(|e| GitError::Other(e.into()))?;
        sqlx::query(
            r#"
            INSERT INTO repository_issue_counters (repository_id, last_number)
            VALUES ($1, 1)
            ON CONFLICT (repository_id) DO UPDATE
              SET last_number = repository_issue_counters.last_number + 1
            "#,
        )
        .bind(repository_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        let (n,): (i32,) = sqlx::query_as(
            "SELECT last_number FROM repository_issue_counters WHERE repository_id = $1",
        )
        .bind(repository_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        tx.commit().await.map_err(|e| GitError::Other(e.into()))?;
        Ok(n)
    }

    async fn insert(&self, issue: &Issue) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO issues (
                id, repository_id, number, title, body, author_id, state,
                closed_at, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            "#,
        )
        .bind(issue.id.as_uuid())
        .bind(issue.repository_id.as_uuid())
        .bind(issue.number)
        .bind(&issue.title)
        .bind(&issue.body)
        .bind(issue.author_id.as_uuid())
        .bind(issue.state.as_str())
        .bind(issue.closed_at)
        .bind(issue.created_at)
        .bind(issue.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(())
    }

    async fn get(&self, id: IssueId) -> Result<Option<Issue>> {
        let row = sqlx::query_as::<_, IssueRow>(&format!(
            "SELECT {ISSUE_COLS} FROM issues WHERE id = $1"
        ))
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
    ) -> Result<Option<Issue>> {
        let row = sqlx::query_as::<_, IssueRow>(&format!(
            "SELECT {ISSUE_COLS} FROM issues WHERE repository_id = $1 AND number = $2"
        ))
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
        state: Option<IssueState>,
        limit: u32,
    ) -> Result<Vec<Issue>> {
        let rows = if let Some(st) = state {
            sqlx::query_as::<_, IssueRow>(&format!(
                "SELECT {ISSUE_COLS} FROM issues
                 WHERE repository_id = $1 AND state = $2
                 ORDER BY number DESC LIMIT $3"
            ))
            .bind(repository_id.as_uuid())
            .bind(st.as_str())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, IssueRow>(&format!(
                "SELECT {ISSUE_COLS} FROM issues
                 WHERE repository_id = $1
                 ORDER BY number DESC LIMIT $2"
            ))
            .bind(repository_id.as_uuid())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update(&self, issue: &Issue) -> Result<()> {
        let res = sqlx::query(
            r#"
            UPDATE issues SET
                title = $2, body = $3, state = $4, closed_at = $5, updated_at = $6
            WHERE id = $1
            "#,
        )
        .bind(issue.id.as_uuid())
        .bind(&issue.title)
        .bind(&issue.body)
        .bind(issue.state.as_str())
        .bind(issue.closed_at)
        .bind(issue.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        if res.rows_affected() == 0 {
            return Err(GitError::NotFound);
        }
        Ok(())
    }

    async fn insert_comment(&self, c: &IssueComment) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO issue_comments (id, issue_id, author_id, body, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6)
            "#,
        )
        .bind(c.id.as_uuid())
        .bind(c.issue_id.as_uuid())
        .bind(c.author_id.as_uuid())
        .bind(&c.body)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(())
    }

    async fn list_comments(&self, issue_id: IssueId, limit: u32) -> Result<Vec<IssueComment>> {
        let rows = sqlx::query_as::<_, CommentRow>(
            r#"
            SELECT id, issue_id, author_id, body, created_at, updated_at
            FROM issue_comments
            WHERE issue_id = $1
            ORDER BY created_at ASC
            LIMIT $2
            "#,
        )
        .bind(issue_id.as_uuid())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

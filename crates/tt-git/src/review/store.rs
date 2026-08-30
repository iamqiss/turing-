use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tt_core::domain::{Review, ReviewState};
use tt_core::ids::{PrincipalId, PullRequestId, ReviewId};
use uuid::Uuid;

use crate::error::{GitError, Result};

#[async_trait]
pub trait ReviewStore: Send + Sync {
    async fn insert(&self, review: &Review) -> Result<()>;
    async fn list(&self, pull_request_id: PullRequestId) -> Result<Vec<Review>>;
    async fn count_approvals(&self, pull_request_id: PullRequestId) -> Result<u32>;
}

#[derive(Clone)]
pub struct PostgresReviewStore {
    pool: PgPool,
}

impl PostgresReviewStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct ReviewRow {
    id: Uuid,
    pull_request_id: Uuid,
    author_id: Uuid,
    state: String,
    body: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<ReviewRow> for Review {
    fn from(r: ReviewRow) -> Self {
        Review {
            id: ReviewId::from_uuid(r.id),
            pull_request_id: PullRequestId::from_uuid(r.pull_request_id),
            author_id: PrincipalId::from_uuid(r.author_id),
            state: ReviewState::parse(&r.state).unwrap_or(ReviewState::Comment),
            body: r.body,
            created_at: r.created_at,
        }
    }
}

#[async_trait]
impl ReviewStore for PostgresReviewStore {
    async fn insert(&self, review: &Review) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO reviews (id, pull_request_id, author_id, state, body, created_at)
            VALUES ($1,$2,$3,$4,$5,$6)
            "#,
        )
        .bind(review.id.as_uuid())
        .bind(review.pull_request_id.as_uuid())
        .bind(review.author_id.as_uuid())
        .bind(review.state.as_str())
        .bind(&review.body)
        .bind(review.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(())
    }

    async fn list(&self, pull_request_id: PullRequestId) -> Result<Vec<Review>> {
        let rows = sqlx::query_as::<_, ReviewRow>(
            r#"
            SELECT id, pull_request_id, author_id, state, body, created_at
            FROM reviews WHERE pull_request_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(pull_request_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn count_approvals(&self, pull_request_id: PullRequestId) -> Result<u32> {
        // Latest review per author; count those that are approve
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint FROM (
                SELECT DISTINCT ON (author_id) state
                FROM reviews
                WHERE pull_request_id = $1
                ORDER BY author_id, created_at DESC
            ) latest
            WHERE state = 'approve'
            "#,
        )
        .bind(pull_request_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(n.max(0) as u32)
    }
}

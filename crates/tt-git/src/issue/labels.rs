//! Repository labels + issue assignment.

use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use tt_core::domain::Label;
use tt_core::ids::{IssueId, LabelId, RepositoryId};
use uuid::Uuid;

use crate::error::{GitError, Result};

#[async_trait]
pub trait LabelStore: Send + Sync {
    async fn create(&self, label: &Label) -> Result<()>;
    async fn list(&self, repository_id: RepositoryId) -> Result<Vec<Label>>;
    async fn get_by_name(
        &self,
        repository_id: RepositoryId,
        name: &str,
    ) -> Result<Option<Label>>;
    async fn delete(&self, id: LabelId) -> Result<()>;
    async fn set_issue_labels(&self, issue_id: IssueId, label_ids: &[LabelId]) -> Result<()>;
    async fn labels_for_issue(&self, issue_id: IssueId) -> Result<Vec<Label>>;
}

#[derive(Clone)]
pub struct PostgresLabelStore {
    pool: PgPool,
}

impl PostgresLabelStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct LabelRow {
    id: Uuid,
    repository_id: Uuid,
    name: String,
    color: String,
    description: Option<String>,
}

impl From<LabelRow> for Label {
    fn from(r: LabelRow) -> Self {
        Label {
            id: LabelId::from_uuid(r.id),
            repository_id: RepositoryId::from_uuid(r.repository_id),
            name: r.name,
            color: r.color,
            description: r.description,
        }
    }
}

#[async_trait]
impl LabelStore for PostgresLabelStore {
    async fn create(&self, label: &Label) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO labels (id, repository_id, name, color, description)
            VALUES ($1,$2,$3,$4,$5)
            "#,
        )
        .bind(label.id.as_uuid())
        .bind(label.repository_id.as_uuid())
        .bind(&label.name)
        .bind(&label.color)
        .bind(&label.description)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("unique") || s.contains("duplicate") {
                GitError::AlreadyExists
            } else {
                GitError::Other(e.into())
            }
        })?;
        Ok(())
    }

    async fn list(&self, repository_id: RepositoryId) -> Result<Vec<Label>> {
        let rows = sqlx::query_as::<_, LabelRow>(
            "SELECT id, repository_id, name, color, description FROM labels WHERE repository_id = $1 ORDER BY name",
        )
        .bind(repository_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_by_name(
        &self,
        repository_id: RepositoryId,
        name: &str,
    ) -> Result<Option<Label>> {
        let row = sqlx::query_as::<_, LabelRow>(
            "SELECT id, repository_id, name, color, description FROM labels WHERE repository_id = $1 AND name = $2",
        )
        .bind(repository_id.as_uuid())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn delete(&self, id: LabelId) -> Result<()> {
        let res = sqlx::query("DELETE FROM labels WHERE id = $1")
            .bind(id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| GitError::Other(e.into()))?;
        if res.rows_affected() == 0 {
            return Err(GitError::NotFound);
        }
        Ok(())
    }

    async fn set_issue_labels(&self, issue_id: IssueId, label_ids: &[LabelId]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| GitError::Other(e.into()))?;
        sqlx::query("DELETE FROM issue_labels WHERE issue_id = $1")
            .bind(issue_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| GitError::Other(e.into()))?;
        for lid in label_ids {
            sqlx::query(
                "INSERT INTO issue_labels (issue_id, label_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
            )
            .bind(issue_id.as_uuid())
            .bind(lid.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| GitError::Other(e.into()))?;
        }
        tx.commit().await.map_err(|e| GitError::Other(e.into()))?;
        Ok(())
    }

    async fn labels_for_issue(&self, issue_id: IssueId) -> Result<Vec<Label>> {
        let rows = sqlx::query_as::<_, LabelRow>(
            r#"
            SELECT l.id, l.repository_id, l.name, l.color, l.description
            FROM labels l
            JOIN issue_labels il ON il.label_id = l.id
            WHERE il.issue_id = $1
            ORDER BY l.name
            "#,
        )
        .bind(issue_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

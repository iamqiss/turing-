//! Persistence for repository metadata.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tt_core::domain::Repository;
use tt_core::ids::{PrincipalId, RepositoryId, WorkspaceId};
use uuid::Uuid;

use crate::error::{GitError, Result};

#[async_trait]
pub trait RepoStore: Send + Sync {
    async fn create(&self, repo: &Repository) -> Result<()>;
    async fn get(&self, id: RepositoryId) -> Result<Option<Repository>>;
    async fn get_by_name(&self, workspace_id: WorkspaceId, name: &str) -> Result<Option<Repository>>;
    async fn list_by_workspace(&self, workspace_id: WorkspaceId) -> Result<Vec<Repository>>;
    async fn delete(&self, id: RepositoryId) -> Result<()>;
}

#[derive(Clone)]
pub struct PostgresRepoStore {
    pool: PgPool,
}

impl PostgresRepoStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct RepoRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    description: Option<String>,
    default_branch: String,
    is_private: bool,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<RepoRow> for Repository {
    fn from(r: RepoRow) -> Self {
        Repository {
            id: RepositoryId::from_uuid(r.id),
            workspace_id: WorkspaceId::from_uuid(r.workspace_id),
            name: r.name,
            description: r.description,
            default_branch: r.default_branch,
            is_private: r.is_private,
            created_by: PrincipalId::from_uuid(r.created_by),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[async_trait]
impl RepoStore for PostgresRepoStore {
    async fn create(&self, repo: &Repository) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO repositories (
                id, workspace_id, name, description, default_branch,
                is_private, created_by, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
            "#,
        )
        .bind(repo.id.as_uuid())
        .bind(repo.workspace_id.as_uuid())
        .bind(&repo.name)
        .bind(&repo.description)
        .bind(&repo.default_branch)
        .bind(repo.is_private)
        .bind(repo.created_by.as_uuid())
        .bind(repo.created_at)
        .bind(repo.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db) = e {
                if db.constraint().is_some() {
                    return GitError::AlreadyExists;
                }
            }
            GitError::Other(e.into())
        })?;
        Ok(())
    }

    async fn get(&self, id: RepositoryId) -> Result<Option<Repository>> {
        let row = sqlx::query_as::<_, RepoRow>(
            r#"
            SELECT id, workspace_id, name, description, default_branch,
                   is_private, created_by, created_at, updated_at
            FROM repositories WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn get_by_name(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<Option<Repository>> {
        let row = sqlx::query_as::<_, RepoRow>(
            r#"
            SELECT id, workspace_id, name, description, default_branch,
                   is_private, created_by, created_at, updated_at
            FROM repositories
            WHERE workspace_id = $1 AND name = $2
            "#,
        )
        .bind(workspace_id.as_uuid())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_by_workspace(&self, workspace_id: WorkspaceId) -> Result<Vec<Repository>> {
        let rows = sqlx::query_as::<_, RepoRow>(
            r#"
            SELECT id, workspace_id, name, description, default_branch,
                   is_private, created_by, created_at, updated_at
            FROM repositories
            WHERE workspace_id = $1
            ORDER BY name
            "#,
        )
        .bind(workspace_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete(&self, id: RepositoryId) -> Result<()> {
        let res = sqlx::query("DELETE FROM repositories WHERE id = $1")
            .bind(id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| GitError::Other(e.into()))?;
        if res.rows_affected() == 0 {
            return Err(GitError::NotFound);
        }
        Ok(())
    }
}

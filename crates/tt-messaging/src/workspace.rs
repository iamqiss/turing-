//! Workspaces and membership.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tracing::info;
use tt_core::domain::{Workspace, WorkspaceMember, WorkspaceRole};
use tt_core::ids::{OrgId, PrincipalId, WorkspaceId};
use uuid::Uuid;

use crate::error::{MessagingError, Result};

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn create(&self, ws: &Workspace, owner_role: WorkspaceRole) -> Result<()>;
    async fn get(&self, id: WorkspaceId) -> Result<Option<Workspace>>;
    async fn get_by_slug(&self, slug: &str) -> Result<Option<Workspace>>;
    async fn list_for_principal(&self, principal_id: PrincipalId) -> Result<Vec<Workspace>>;
    async fn add_member(&self, member: &WorkspaceMember) -> Result<()>;
    async fn is_member(&self, workspace_id: WorkspaceId, principal_id: PrincipalId) -> Result<bool>;
}

#[derive(Clone)]
pub struct PostgresWorkspaceStore {
    pool: PgPool,
}

impl PostgresWorkspaceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct WsRow {
    id: Uuid,
    org_id: Option<Uuid>,
    name: String,
    slug: String,
    description: Option<String>,
    owner_id: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<WsRow> for Workspace {
    fn from(r: WsRow) -> Self {
        Workspace {
            id: WorkspaceId::from_uuid(r.id),
            org_id: r.org_id.map(OrgId::from_uuid),
            name: r.name,
            slug: r.slug,
            description: r.description,
            owner_id: PrincipalId::from_uuid(r.owner_id),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

fn role_str(r: WorkspaceRole) -> &'static str {
    match r {
        WorkspaceRole::Owner => "owner",
        WorkspaceRole::Admin => "admin",
        WorkspaceRole::Member => "member",
        WorkspaceRole::Guest => "guest",
    }
}

#[async_trait]
impl WorkspaceStore for PostgresWorkspaceStore {
    async fn create(&self, ws: &Workspace, owner_role: WorkspaceRole) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| MessagingError::Other(e.into()))?;
        sqlx::query(
            r#"
            INSERT INTO workspaces (id, org_id, name, slug, description, owner_id, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            "#,
        )
        .bind(ws.id.as_uuid())
        .bind(ws.org_id.as_ref().map(|id| *id.as_uuid()))
        .bind(&ws.name)
        .bind(&ws.slug)
        .bind(&ws.description)
        .bind(ws.owner_id.as_uuid())
        .bind(ws.created_at)
        .bind(ws.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db) = e {
                if db.constraint().is_some() {
                    return MessagingError::AlreadyExists;
                }
            }
            MessagingError::Other(e.into())
        })?;

        sqlx::query(
            r#"
            INSERT INTO workspace_members (workspace_id, principal_id, role, joined_at)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(ws.id.as_uuid())
        .bind(ws.owner_id.as_uuid())
        .bind(role_str(owner_role))
        .bind(ws.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;

        tx.commit().await.map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn get(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        let row = sqlx::query_as::<_, WsRow>(
            r#"SELECT id, org_id, name, slug, description, owner_id, created_at, updated_at
               FROM workspaces WHERE id = $1"#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<Workspace>> {
        let row = sqlx::query_as::<_, WsRow>(
            r#"SELECT id, org_id, name, slug, description, owner_id, created_at, updated_at
               FROM workspaces WHERE lower(slug) = lower($1)"#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_for_principal(&self, principal_id: PrincipalId) -> Result<Vec<Workspace>> {
        let rows = sqlx::query_as::<_, WsRow>(
            r#"
            SELECT w.id, w.org_id, w.name, w.slug, w.description, w.owner_id, w.created_at, w.updated_at
            FROM workspaces w
            INNER JOIN workspace_members m ON m.workspace_id = w.id
            WHERE m.principal_id = $1
            ORDER BY w.name
            "#,
        )
        .bind(principal_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn add_member(&self, member: &WorkspaceMember) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO workspace_members (workspace_id, principal_id, role, joined_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(member.workspace_id.as_uuid())
        .bind(member.principal_id.as_uuid())
        .bind(role_str(member.role))
        .bind(member.joined_at)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn is_member(&self, workspace_id: WorkspaceId, principal_id: PrincipalId) -> Result<bool> {
        let row: (bool,) = sqlx::query_as(
            r#"SELECT EXISTS(
                SELECT 1 FROM workspace_members
                WHERE workspace_id = $1 AND principal_id = $2
            )"#,
        )
        .bind(workspace_id.as_uuid())
        .bind(principal_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.0)
    }
}

pub struct WorkspaceService {
    store: Arc<dyn WorkspaceStore>,
}

impl WorkspaceService {
    pub fn new(store: Arc<dyn WorkspaceStore>) -> Self {
        Self { store }
    }

    pub fn validate_slug(slug: &str) -> Result<()> {
        if slug.is_empty() || slug.len() > 64 {
            return Err(MessagingError::InvalidArgument(
                "slug must be 1–64 characters".into(),
            ));
        }
        if !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(MessagingError::InvalidArgument(
                "slug: lowercase, digits, hyphens only".into(),
            ));
        }
        Ok(())
    }

    pub async fn create(
        &self,
        name: String,
        slug: String,
        description: Option<String>,
        owner_id: PrincipalId,
    ) -> Result<Workspace> {
        Self::validate_slug(&slug)?;
        if name.trim().is_empty() {
            return Err(MessagingError::InvalidArgument("name required".into()));
        }
        if self.store.get_by_slug(&slug).await?.is_some() {
            return Err(MessagingError::AlreadyExists);
        }

        let now = Utc::now();
        let ws = Workspace {
            id: WorkspaceId::new(),
            org_id: None,
            name,
            slug,
            description,
            owner_id,
            created_at: now,
            updated_at: now,
        };
        self.store.create(&ws, WorkspaceRole::Owner).await?;
        info!(workspace_id = %ws.id, slug = %ws.slug, "workspace created");
        Ok(ws)
    }

    pub async fn create_in_org(
        &self,
        name: String,
        slug: String,
        description: Option<String>,
        owner_id: PrincipalId,
        org_id: OrgId,
    ) -> Result<Workspace> {
        Self::validate_slug(&slug)?;
        if name.trim().is_empty() {
            return Err(MessagingError::InvalidArgument("name required".into()));
        }
        if self.store.get_by_slug(&slug).await?.is_some() {
            return Err(MessagingError::AlreadyExists);
        }
        let now = Utc::now();
        let ws = Workspace {
            id: WorkspaceId::new(),
            org_id: Some(org_id),
            name,
            slug,
            description,
            owner_id,
            created_at: now,
            updated_at: now,
        };
        self.store.create(&ws, WorkspaceRole::Owner).await?;
        info!(workspace_id = %ws.id, org_id = %org_id, "workspace created in org");
        Ok(ws)
    }

    pub async fn get(&self, id: WorkspaceId) -> Result<Workspace> {
        self.store.get(id).await?.ok_or(MessagingError::NotFound)
    }

    pub async fn list_for_principal(&self, principal_id: PrincipalId) -> Result<Vec<Workspace>> {
        self.store.list_for_principal(principal_id).await
    }

    pub async fn ensure_member(
        &self,
        workspace_id: WorkspaceId,
        principal_id: PrincipalId,
    ) -> Result<()> {
        if !self.store.is_member(workspace_id, principal_id).await? {
            return Err(MessagingError::PermissionDenied(
                "not a member of this workspace".into(),
            ));
        }
        Ok(())
    }
}

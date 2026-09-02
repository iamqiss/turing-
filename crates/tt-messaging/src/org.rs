//! Organizations: seats, membership, invitations.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use sqlx::{FromRow, PgPool};
use tracing::info;
use tt_core::domain::{
    MergeMethod, Org, OrgBillingSnapshot, OrgInvitation, OrgMember, OrgRole,
    RepositorySettings, SeatStatus,
};
use tt_core::ids::{OrgId, OrgInvitationId, PrincipalId, WorkspaceId};
use uuid::Uuid;

use crate::error::{MessagingError, Result};

#[async_trait]
pub trait OrgStore: Send + Sync {
    async fn insert_org(&self, org: &Org) -> Result<()>;
    async fn get(&self, id: OrgId) -> Result<Option<Org>>;
    async fn get_by_slug(&self, slug: &str) -> Result<Option<Org>>;
    async fn list_for_principal(&self, principal_id: PrincipalId) -> Result<Vec<Org>>;
    async fn add_member(&self, member: &OrgMember) -> Result<()>;
    async fn get_member(
        &self,
        org_id: OrgId,
        principal_id: PrincipalId,
    ) -> Result<Option<OrgMember>>;
    async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>>;
    async fn delete_member(&self, org_id: OrgId, principal_id: PrincipalId) -> Result<()>;
    async fn count_seats_used(&self, org_id: OrgId) -> Result<u32>;
    async fn update_seat_limit(&self, org_id: OrgId, limit: u32) -> Result<()>;
    async fn insert_invitation(&self, inv: &OrgInvitation) -> Result<()>;
    async fn get_invitation_by_token(&self, token: &str) -> Result<Option<OrgInvitation>>;
    async fn delete_invitation(&self, id: OrgInvitationId) -> Result<()>;
    async fn list_invitations(&self, org_id: OrgId) -> Result<Vec<OrgInvitation>>;
    async fn set_workspace_org(&self, workspace_id: WorkspaceId, org_id: OrgId) -> Result<()>;
}

#[derive(Clone)]
pub struct PostgresOrgStore {
    pool: PgPool,
}

impl PostgresOrgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct OrgRow {
    id: Uuid,
    name: String,
    slug: String,
    seat_limit: i32,
    billing_customer_id: Option<String>,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

impl From<OrgRow> for Org {
    fn from(r: OrgRow) -> Self {
        Org {
            id: OrgId::from_uuid(r.id),
            name: r.name,
            slug: r.slug,
            seat_limit: r.seat_limit.max(0) as u32,
            billing_customer_id: r.billing_customer_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(FromRow)]
struct MemberRow {
    org_id: Uuid,
    principal_id: Uuid,
    role: String,
    seat_status: String,
    joined_at: chrono::DateTime<Utc>,
}

impl From<MemberRow> for OrgMember {
    fn from(r: MemberRow) -> Self {
        OrgMember {
            org_id: OrgId::from_uuid(r.org_id),
            principal_id: PrincipalId::from_uuid(r.principal_id),
            role: OrgRole::parse(&r.role).unwrap_or(OrgRole::Member),
            seat_status: SeatStatus::parse(&r.seat_status).unwrap_or(SeatStatus::Active),
            joined_at: r.joined_at,
        }
    }
}

#[derive(FromRow)]
struct InvRow {
    id: Uuid,
    org_id: Uuid,
    email: String,
    role: String,
    token: String,
    invited_by: Uuid,
    expires_at: chrono::DateTime<Utc>,
    created_at: chrono::DateTime<Utc>,
}

impl From<InvRow> for OrgInvitation {
    fn from(r: InvRow) -> Self {
        OrgInvitation {
            id: OrgInvitationId::from_uuid(r.id),
            org_id: OrgId::from_uuid(r.org_id),
            email: r.email,
            role: OrgRole::parse(&r.role).unwrap_or(OrgRole::Member),
            token: r.token,
            invited_by: PrincipalId::from_uuid(r.invited_by),
            expires_at: r.expires_at,
            created_at: r.created_at,
        }
    }
}

#[async_trait]
impl OrgStore for PostgresOrgStore {
    async fn insert_org(&self, org: &Org) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO orgs (id, name, slug, seat_limit, billing_customer_id, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(org.id.as_uuid())
        .bind(&org.name)
        .bind(&org.slug)
        .bind(org.seat_limit as i32)
        .bind(&org.billing_customer_id)
        .bind(org.created_at)
        .bind(org.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn get(&self, id: OrgId) -> Result<Option<Org>> {
        let row = sqlx::query_as::<_, OrgRow>(
            "SELECT id, name, slug, seat_limit, billing_customer_id, created_at, updated_at FROM orgs WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<Org>> {
        let row = sqlx::query_as::<_, OrgRow>(
            "SELECT id, name, slug, seat_limit, billing_customer_id, created_at, updated_at FROM orgs WHERE lower(slug) = lower($1)",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_for_principal(&self, principal_id: PrincipalId) -> Result<Vec<Org>> {
        let rows = sqlx::query_as::<_, OrgRow>(
            r#"
            SELECT o.id, o.name, o.slug, o.seat_limit, o.billing_customer_id, o.created_at, o.updated_at
            FROM orgs o
            JOIN org_members m ON m.org_id = o.id
            WHERE m.principal_id = $1
            ORDER BY o.name
            "#,
        )
        .bind(principal_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn add_member(&self, member: &OrgMember) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO org_members (org_id, principal_id, role, seat_status, joined_at)
            VALUES ($1,$2,$3,$4,$5)
            ON CONFLICT (org_id, principal_id) DO UPDATE SET
                role = EXCLUDED.role,
                seat_status = EXCLUDED.seat_status
            "#,
        )
        .bind(member.org_id.as_uuid())
        .bind(member.principal_id.as_uuid())
        .bind(member.role.as_str())
        .bind(member.seat_status.as_str())
        .bind(member.joined_at)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn get_member(
        &self,
        org_id: OrgId,
        principal_id: PrincipalId,
    ) -> Result<Option<OrgMember>> {
        let row = sqlx::query_as::<_, MemberRow>(
            "SELECT org_id, principal_id, role, seat_status, joined_at FROM org_members WHERE org_id = $1 AND principal_id = $2",
        )
        .bind(org_id.as_uuid())
        .bind(principal_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>> {
        let rows = sqlx::query_as::<_, MemberRow>(
            "SELECT org_id, principal_id, role, seat_status, joined_at FROM org_members WHERE org_id = $1 ORDER BY joined_at",
        )
        .bind(org_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    
    async fn delete_member(&self, org_id: OrgId, principal_id: PrincipalId) -> Result<()> {
        sqlx::query("DELETE FROM org_members WHERE org_id = $1 AND principal_id = $2")
            .bind(org_id.as_uuid())
            .bind(principal_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn count_seats_used(&self, org_id: OrgId) -> Result<u32> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint FROM org_members
            WHERE org_id = $1 AND seat_status IN ('active', 'invited')
            "#,
        )
        .bind(org_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(n.max(0) as u32)
    }

    async fn update_seat_limit(&self, org_id: OrgId, limit: u32) -> Result<()> {
        sqlx::query("UPDATE orgs SET seat_limit = $2, updated_at = NOW() WHERE id = $1")
            .bind(org_id.as_uuid())
            .bind(limit as i32)
            .execute(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn insert_invitation(&self, inv: &OrgInvitation) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO org_invitations (id, org_id, email, role, token, invited_by, expires_at, created_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            "#,
        )
        .bind(inv.id.as_uuid())
        .bind(inv.org_id.as_uuid())
        .bind(&inv.email)
        .bind(inv.role.as_str())
        .bind(&inv.token)
        .bind(inv.invited_by.as_uuid())
        .bind(inv.expires_at)
        .bind(inv.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn get_invitation_by_token(&self, token: &str) -> Result<Option<OrgInvitation>> {
        let row = sqlx::query_as::<_, InvRow>(
            "SELECT id, org_id, email, role, token, invited_by, expires_at, created_at FROM org_invitations WHERE token = $1",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn delete_invitation(&self, id: OrgInvitationId) -> Result<()> {
        sqlx::query("DELETE FROM org_invitations WHERE id = $1")
            .bind(id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn list_invitations(&self, org_id: OrgId) -> Result<Vec<OrgInvitation>> {
        let rows = sqlx::query_as::<_, InvRow>(
            "SELECT id, org_id, email, role, token, invited_by, expires_at, created_at FROM org_invitations WHERE org_id = $1 ORDER BY created_at DESC",
        )
        .bind(org_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn set_workspace_org(&self, workspace_id: WorkspaceId, org_id: OrgId) -> Result<()> {
        sqlx::query("UPDATE workspaces SET org_id = $2 WHERE id = $1")
            .bind(workspace_id.as_uuid())
            .bind(org_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }
}

pub struct OrgService {
    store: Arc<dyn OrgStore>,
    pool: PgPool,
}

impl OrgService {
    pub fn new(store: Arc<dyn OrgStore>, pool: PgPool) -> Self {
        Self { store, pool }
    }

    pub async fn create(
        &self,
        name: String,
        slug: String,
        owner_id: PrincipalId,
        seat_limit: u32,
    ) -> Result<Org> {
        let name = name.trim().to_string();
        let slug = slug.trim().to_lowercase();
        if name.is_empty() || slug.is_empty() {
            return Err(MessagingError::InvalidArgument(
                "name and slug required".into(),
            ));
        }
        if !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(MessagingError::InvalidArgument(
                "slug must be lowercase alphanumeric/hyphen".into(),
            ));
        }
        if self.store.get_by_slug(&slug).await?.is_some() {
            return Err(MessagingError::Conflict("org slug already exists".into()));
        }
        let now = Utc::now();
        let org = Org {
            id: OrgId::new(),
            name,
            slug,
            seat_limit: seat_limit.max(1),
            billing_customer_id: None,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_org(&org).await?;
        self.store
            .add_member(&OrgMember {
                org_id: org.id,
                principal_id: owner_id,
                role: OrgRole::Owner,
                seat_status: SeatStatus::Active,
                joined_at: now,
            })
            .await?;
        info!(org_id = %org.id, slug = %org.slug, "org created");
        Ok(org)
    }

    /// Personal org for a principal (bootstrap / first login).
    pub async fn ensure_personal(&self, principal_id: PrincipalId, handle: &str) -> Result<Org> {
        let slug = format!("personal-{}", handle.to_lowercase());
        if let Some(existing) = self.store.get_by_slug(&slug).await? {
            return Ok(existing);
        }
        // also: if already member of any org as owner, skip
        let orgs = self.store.list_for_principal(principal_id).await?;
        if let Some(o) = orgs.first() {
            return Ok(o.clone());
        }
        self.create(
            format!("{}'s org", handle),
            slug,
            principal_id,
            5, // default startup trial seats
        )
        .await
    }

    pub async fn get(&self, id: OrgId) -> Result<Org> {
        self.store.get(id).await?.ok_or(MessagingError::NotFound)
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Org> {
        self.store
            .get_by_slug(slug)
            .await?
            .ok_or(MessagingError::NotFound)
    }

    pub async fn list_mine(&self, principal_id: PrincipalId) -> Result<Vec<Org>> {
        self.store.list_for_principal(principal_id).await
    }

    pub async fn billing(&self, org_id: OrgId) -> Result<OrgBillingSnapshot> {
        let org = self.get(org_id).await?;
        let members = self.store.count_seats_used(org_id).await?;
        let pending = self.store.list_invitations(org_id).await?.len() as u32;
        let used = members + pending;
        Ok(OrgBillingSnapshot {
            org_id,
            seat_limit: org.seat_limit,
            seats_used: used,
            seats_available: org.seat_limit.saturating_sub(used),
        })
    }

    pub async fn require_member(&self, org_id: OrgId, principal_id: PrincipalId) -> Result<()> {
        self.store
            .get_member(org_id, principal_id)
            .await?
            .ok_or(MessagingError::Forbidden("not an org member".into()))?;
        Ok(())
    }

    pub async fn require_admin(&self, org_id: OrgId, principal_id: PrincipalId) -> Result<()> {
        self.require_permission(org_id, principal_id, tt_core::domain::OrgPermission::ManageOrgSettings)
            .await
    }

    pub async fn require_owner(&self, org_id: OrgId, principal_id: PrincipalId) -> Result<()> {
        let m = self
            .store
            .get_member(org_id, principal_id)
            .await?
            .ok_or(MessagingError::Forbidden("not an org member".into()))?;
        if !matches!(m.role, OrgRole::Owner) {
            return Err(MessagingError::Forbidden("org owner required".into()));
        }
        if matches!(m.seat_status, tt_core::domain::SeatStatus::Suspended) {
            return Err(MessagingError::Forbidden("seat suspended".into()));
        }
        Ok(())
    }

    pub async fn require_permission(
        &self,
        org_id: OrgId,
        principal_id: PrincipalId,
        perm: tt_core::domain::OrgPermission,
    ) -> Result<()> {
        let m = self
            .store
            .get_member(org_id, principal_id)
            .await?
            .ok_or(MessagingError::Forbidden("not an org member".into()))?;
        if matches!(m.seat_status, tt_core::domain::SeatStatus::Suspended) {
            return Err(MessagingError::Forbidden("seat suspended".into()));
        }
        if !m.role.allows(perm) {
            return Err(MessagingError::Forbidden(format!(
                "missing permission: {}",
                perm.as_str()
            )));
        }
        Ok(())
    }

    pub async fn member_role(
        &self,
        org_id: OrgId,
        principal_id: PrincipalId,
    ) -> Result<OrgRole> {
        let m = self
            .store
            .get_member(org_id, principal_id)
            .await?
            .ok_or(MessagingError::Forbidden("not an org member".into()))?;
        Ok(m.role)
    }

    /// Active seat only — suspended members cannot mutate compute.
    pub async fn require_active_member(
        &self,
        org_id: OrgId,
        principal_id: PrincipalId,
    ) -> Result<()> {
        let m = self
            .store
            .get_member(org_id, principal_id)
            .await?
            .ok_or(MessagingError::Forbidden("not an org member".into()))?;
        if matches!(m.seat_status, tt_core::domain::SeatStatus::Suspended) {
            return Err(MessagingError::Forbidden("seat suspended".into()));
        }
        Ok(())
    }


    async fn count_owners(&self, org_id: OrgId) -> Result<u32> {
        let members = self.store.list_members(org_id).await?;
        Ok(members
            .iter()
            .filter(|m| matches!(m.role, OrgRole::Owner) && matches!(m.seat_status, tt_core::domain::SeatStatus::Active))
            .count() as u32)
    }

    /// Change a member's role. Cannot leave org without an owner. Cannot escalate above actor.
    pub async fn set_member_role(
        &self,
        org_id: OrgId,
        actor_id: PrincipalId,
        target_id: PrincipalId,
        new_role: OrgRole,
    ) -> Result<OrgMember> {
        self.require_permission(org_id, actor_id, tt_core::domain::OrgPermission::ChangeRoles)
            .await?;
        if matches!(new_role, OrgRole::Owner) {
            self.require_owner(org_id, actor_id).await?;
        }
        let actor = self
            .store
            .get_member(org_id, actor_id)
            .await?
            .ok_or(MessagingError::Forbidden("not an org member".into()))?;
        // Actor cannot grant a role above their own (except owner promoting admin is fine; owner is top)
        if new_role.rank() > actor.role.rank() {
            return Err(MessagingError::Forbidden(
                "cannot grant a role higher than your own".into(),
            ));
        }
        let mut target = self
            .store
            .get_member(org_id, target_id)
            .await?
            .ok_or(MessagingError::NotFound)?;
        if matches!(target.role, OrgRole::Owner) && !matches!(new_role, OrgRole::Owner) {
            if self.count_owners(org_id).await? <= 1 {
                return Err(MessagingError::Conflict(
                    "cannot demote the last owner".into(),
                ));
            }
            // only another owner may demote an owner
            if !matches!(actor.role, OrgRole::Owner) {
                return Err(MessagingError::Forbidden("only owner can demote an owner".into()));
            }
        }
        target.role = new_role;
        self.store.add_member(&target).await?;
        Ok(target)
    }

    pub async fn transfer_ownership(
        &self,
        org_id: OrgId,
        actor_id: PrincipalId,
        new_owner_id: PrincipalId,
    ) -> Result<()> {
        self.require_owner(org_id, actor_id).await?;
        if actor_id == new_owner_id {
            return Ok(());
        }
        let mut target = self
            .store
            .get_member(org_id, new_owner_id)
            .await?
            .ok_or(MessagingError::NotFound)?;
        let mut actor = self
            .store
            .get_member(org_id, actor_id)
            .await?
            .ok_or(MessagingError::Forbidden("not an org member".into()))?;
        target.role = OrgRole::Owner;
        actor.role = OrgRole::Admin;
        self.store.add_member(&target).await?;
        self.store.add_member(&actor).await?;
        Ok(())
    }

    pub async fn remove_member(
        &self,
        org_id: OrgId,
        actor_id: PrincipalId,
        target_id: PrincipalId,
    ) -> Result<()> {
        self.require_permission(org_id, actor_id, tt_core::domain::OrgPermission::RemoveMembers)
            .await?;
        let target = self
            .store
            .get_member(org_id, target_id)
            .await?
            .ok_or(MessagingError::NotFound)?;
        if matches!(target.role, OrgRole::Owner) {
            if self.count_owners(org_id).await? <= 1 {
                return Err(MessagingError::Conflict(
                    "cannot remove the last owner".into(),
                ));
            }
            self.require_owner(org_id, actor_id).await?;
        }
        self.store.delete_member(org_id, target_id).await?;
        Ok(())
    }

    pub async fn invite(
        &self,
        org_id: OrgId,
        actor_id: PrincipalId,
        email: String,
        role: OrgRole,
    ) -> Result<OrgInvitation> {
        self.require_admin(org_id, actor_id).await?;
        let email = email.trim().to_lowercase();
        if email.is_empty() || !email.contains('@') {
            return Err(MessagingError::InvalidArgument("valid email required".into()));
        }
        if matches!(role, OrgRole::Owner) {
            return Err(MessagingError::InvalidArgument(
                "cannot invite as owner".into(),
            ));
        }
        let snap = self.billing(org_id).await?;
        if snap.seats_available == 0 {
            return Err(MessagingError::Conflict(format!(
                "no seats available ({}/{})",
                snap.seats_used, snap.seat_limit
            )));
        }
        let now = Utc::now();
        let inv = OrgInvitation {
            id: OrgInvitationId::new(),
            org_id,
            email,
            role,
            token: Uuid::new_v4().to_string().replace('-', ""),
            invited_by: actor_id,
            expires_at: now + Duration::days(14),
            created_at: now,
        };
        self.store.insert_invitation(&inv).await?;
        // Reserve seat as invited placeholder — only after accept for principal;
        // seat check on invite uses count of active+invited members only.
        // Invitation itself doesn't add a member until accept.
        info!(org_id = %org_id, email = %inv.email, "org invitation created");
        Ok(inv)
    }

    pub async fn accept_invitation(
        &self,
        token: &str,
        principal_id: PrincipalId,
    ) -> Result<OrgMember> {
        let inv = self
            .store
            .get_invitation_by_token(token)
            .await?
            .ok_or(MessagingError::NotFound)?;
        if inv.expires_at < Utc::now() {
            return Err(MessagingError::InvalidArgument("invitation expired".into()));
        }
        let snap = self.billing(inv.org_id).await?;
        if snap.seats_available == 0 {
            return Err(MessagingError::Conflict("no seats available".into()));
        }
        if self
            .store
            .get_member(inv.org_id, principal_id)
            .await?
            .is_some()
        {
            let _ = self.store.delete_invitation(inv.id).await;
            return self
                .store
                .get_member(inv.org_id, principal_id)
                .await?
                .ok_or(MessagingError::NotFound);
        }
        let member = OrgMember {
            org_id: inv.org_id,
            principal_id,
            role: inv.role,
            seat_status: SeatStatus::Active,
            joined_at: Utc::now(),
        };
        self.store.add_member(&member).await?;
        let _ = self.store.delete_invitation(inv.id).await;
        Ok(member)
    }

    pub async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>> {
        self.store.list_members(org_id).await
    }

    pub async fn set_seat_limit(
        &self,
        org_id: OrgId,
        actor_id: PrincipalId,
        limit: u32,
    ) -> Result<Org> {
        self.require_admin(org_id, actor_id).await?;
        let used = self.store.count_seats_used(org_id).await?;
        if limit < used {
            return Err(MessagingError::InvalidArgument(format!(
                "seat_limit {limit} < seats in use {used}"
            )));
        }
        self.store.update_seat_limit(org_id, limit.max(1)).await?;
        self.get(org_id).await
    }

    pub async fn list_invitations(&self, org_id: OrgId) -> Result<Vec<OrgInvitation>> {
        self.store.list_invitations(org_id).await
    }

    pub async fn attach_workspace(
        &self,
        org_id: OrgId,
        workspace_id: WorkspaceId,
        actor_id: PrincipalId,
    ) -> Result<()> {
        self.require_admin(org_id, actor_id).await?;
        self.store.set_workspace_org(workspace_id, org_id).await
    }
}

// --- Org default policies + SSO ---

#[derive(Debug, Clone)]
pub struct OrgPolicy {
    pub org_id: OrgId,
    pub allowed_merge_methods: Vec<MergeMethod>,
    pub default_merge_method: MergeMethod,
    pub delete_branch_on_merge: bool,
    pub require_up_to_date: bool,
    pub restrict_merge_to_admins: bool,
    pub required_approvals: u32,
}

impl OrgPolicy {
    pub fn defaults(org_id: OrgId) -> Self {
        Self {
            org_id,
            allowed_merge_methods: vec![
                MergeMethod::Merge,
                MergeMethod::Squash,
                MergeMethod::Rebase,
            ],
            default_merge_method: MergeMethod::Merge,
            delete_branch_on_merge: false,
            require_up_to_date: false,
            restrict_merge_to_admins: false,
            required_approvals: 0,
        }
    }

    pub fn to_repo_settings(&self, repository_id: tt_core::ids::RepositoryId) -> RepositorySettings {
        RepositorySettings {
            repository_id,
            allowed_merge_methods: self.allowed_merge_methods.clone(),
            default_merge_method: self.default_merge_method,
            delete_branch_on_merge: self.delete_branch_on_merge,
            require_up_to_date: self.require_up_to_date,
            restrict_merge_to_admins: self.restrict_merge_to_admins,
            required_approvals: self.required_approvals,
            block_secret_push: true,
            secret_push_allowlist: vec![],
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrgSsoConfig {
    pub org_id: OrgId,
    pub enabled: bool,
    pub provider: String,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    /// Never returned to clients in full — write-only
    pub client_secret_set: bool,
    pub domains: Vec<String>,
}

impl OrgService {
    pub async fn get_policy(&self, org_id: OrgId) -> Result<OrgPolicy> {
        let row: Option<(Vec<String>, String, bool, bool, bool, i32)> = sqlx::query_as(
            r#"
            SELECT allowed_merge_methods, default_merge_method, delete_branch_on_merge,
                   require_up_to_date, restrict_merge_to_admins, required_approvals
            FROM org_settings WHERE org_id = $1
            "#,
        )
        .bind(org_id.as_uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;

        Ok(match row {
            Some((allowed, default, del, up, admins, appr)) => OrgPolicy {
                org_id,
                allowed_merge_methods: allowed
                    .into_iter()
                    .filter_map(|s| MergeMethod::parse(&s))
                    .collect(),
                default_merge_method: MergeMethod::parse(&default).unwrap_or(MergeMethod::Merge),
                delete_branch_on_merge: del,
                require_up_to_date: up,
                restrict_merge_to_admins: admins,
                required_approvals: appr.max(0) as u32,
            },
            None => OrgPolicy::defaults(org_id),
        })
    }

    pub async fn upsert_policy(&self, policy: &OrgPolicy) -> Result<()> {
        let allowed: Vec<String> = policy
            .allowed_merge_methods
            .iter()
            .map(|m| m.as_str().to_string())
            .collect();
        sqlx::query(
            r#"
            INSERT INTO org_settings (
                org_id, allowed_merge_methods, default_merge_method,
                delete_branch_on_merge, require_up_to_date, restrict_merge_to_admins,
                required_approvals
            ) VALUES ($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT (org_id) DO UPDATE SET
                allowed_merge_methods = EXCLUDED.allowed_merge_methods,
                default_merge_method = EXCLUDED.default_merge_method,
                delete_branch_on_merge = EXCLUDED.delete_branch_on_merge,
                require_up_to_date = EXCLUDED.require_up_to_date,
                restrict_merge_to_admins = EXCLUDED.restrict_merge_to_admins,
                required_approvals = EXCLUDED.required_approvals,
                updated_at = NOW()
            "#,
        )
        .bind(policy.org_id.as_uuid())
        .bind(&allowed)
        .bind(policy.default_merge_method.as_str())
        .bind(policy.delete_branch_on_merge)
        .bind(policy.require_up_to_date)
        .bind(policy.restrict_merge_to_admins)
        .bind(policy.required_approvals as i32)
        .execute(self.pool())
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    pub async fn get_sso(&self, org_id: OrgId) -> Result<OrgSsoConfig> {
        let row: Option<(bool, String, Option<String>, Option<String>, Option<String>, Vec<String>)> =
            sqlx::query_as(
                r#"
                SELECT enabled, provider, issuer_url, client_id, client_secret, domains
                FROM org_sso_configs WHERE org_id = $1
                "#,
            )
            .bind(org_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(match row {
            Some((enabled, provider, issuer_url, client_id, secret, domains)) => OrgSsoConfig {
                org_id,
                enabled,
                provider,
                issuer_url,
                client_id,
                client_secret_set: secret.as_ref().map(|s| !s.is_empty()).unwrap_or(false),
                domains,
            },
            None => OrgSsoConfig {
                org_id,
                enabled: false,
                provider: "oidc".into(),
                issuer_url: None,
                client_id: None,
                client_secret_set: false,
                domains: vec![],
            },
        })
    }

    pub async fn upsert_sso(
        &self,
        org_id: OrgId,
        enabled: bool,
        issuer_url: Option<String>,
        client_id: Option<String>,
        client_secret: Option<String>,
        domains: Vec<String>,
    ) -> Result<OrgSsoConfig> {
        // Keep existing secret if not provided
        let existing = self.get_sso(org_id).await?;
        let secret = match client_secret {
            Some(s) if !s.is_empty() => Some(s),
            _ => None,
        };
        sqlx::query(
            r#"
            INSERT INTO org_sso_configs (org_id, enabled, provider, issuer_url, client_id, client_secret, domains)
            VALUES ($1,$2,'oidc',$3,$4,COALESCE($5, ''),$6)
            ON CONFLICT (org_id) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                issuer_url = EXCLUDED.issuer_url,
                client_id = EXCLUDED.client_id,
                client_secret = CASE
                    WHEN $5 IS NOT NULL AND $5 <> '' THEN $5
                    ELSE org_sso_configs.client_secret
                END,
                domains = EXCLUDED.domains,
                updated_at = NOW()
            "#,
        )
        .bind(org_id.as_uuid())
        .bind(enabled)
        .bind(&issuer_url)
        .bind(&client_id)
        .bind(&secret)
        .bind(&domains)
        .execute(self.pool())
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        let _ = existing;
        self.get_sso(org_id).await
    }

    pub async fn set_billing_from_stripe(
        &self,
        customer_id: &str,
        subscription_id: Option<&str>,
        seat_limit: u32,
        status: &str,
    ) -> Result<Option<Org>> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM orgs WHERE billing_customer_id = $1",
        )
        .bind(customer_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        let Some((id,)) = row else {
            return Ok(None);
        };
        sqlx::query(
            r#"
            UPDATE orgs SET
                seat_limit = $2,
                stripe_subscription_id = COALESCE($3, stripe_subscription_id),
                billing_status = $4,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(seat_limit as i32)
        .bind(subscription_id)
        .bind(status)
        .execute(self.pool())
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        self.store.get(OrgId::from_uuid(id)).await
    }

    pub async fn link_stripe_customer(
        &self,
        org_id: OrgId,
        customer_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE orgs SET billing_customer_id = $2, billing_status = 'linked', updated_at = NOW() WHERE id = $1",
        )
        .bind(org_id.as_uuid())
        .bind(customer_id)
        .execute(self.pool())
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    fn pool(&self) -> &PgPool {
        &self.pool
    }
}

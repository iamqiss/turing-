use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{OrgId, OrgInvitationId, PrincipalId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    Owner,
    Admin,
    Member,
    Billing,
}

impl OrgRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Billing => "billing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            "billing" => Some(Self::Billing),
            _ => None,
        }
    }

    pub fn is_adminish(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Hierarchy for comparisons (owner > admin > billing > member).
    pub fn rank(self) -> u8 {
        match self {
            Self::Owner => 40,
            Self::Admin => 30,
            Self::Billing => 20,
            Self::Member => 10,
        }
    }

    pub fn at_least(self, other: Self) -> bool {
        self.rank() >= other.rank()
    }

    /// Capability matrix — single source of truth for org RBAC.
    pub fn allows(self, perm: OrgPermission) -> bool {
        use OrgPermission::*;
        match perm {
            // membership
            ViewOrg | ViewMembers => true, // any role
            InviteMembers | RemoveMembers | ChangeRoles => self.is_adminish(),
            TransferOwnership => matches!(self, Self::Owner),
            // billing
            ViewBilling => matches!(self, Self::Owner | Self::Admin | Self::Billing),
            ManageBilling => matches!(self, Self::Owner | Self::Admin | Self::Billing),
            // settings / sso / policies
            ManageOrgSettings | ManageSso | ManagePolicies => self.is_adminish(),
            // workspaces & repos
            CreateWorkspace => self.is_adminish(),
            // compute
            CreateWorkload | StartWorkload | StopWorkload => {
                matches!(self, Self::Owner | Self::Admin | Self::Member)
            }
            DestroyWorkload | ManageQuotas => self.is_adminish(),
            // secrets
            ReadSecrets => matches!(self, Self::Owner | Self::Admin | Self::Member),
            WriteSecrets => self.is_adminish(),
            // agents
            RunAgentSafe => matches!(self, Self::Owner | Self::Admin | Self::Member),
            RunAgentDangerous => self.is_adminish(),
        }
    }
}

/// Fine-grained org permissions (checked via `OrgRole::allows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgPermission {
    ViewOrg,
    ViewMembers,
    InviteMembers,
    RemoveMembers,
    ChangeRoles,
    TransferOwnership,
    ViewBilling,
    ManageBilling,
    ManageOrgSettings,
    ManageSso,
    ManagePolicies,
    CreateWorkspace,
    CreateWorkload,
    StartWorkload,
    StopWorkload,
    DestroyWorkload,
    ManageQuotas,
    ReadSecrets,
    WriteSecrets,
    RunAgentSafe,
    RunAgentDangerous,
}

impl OrgPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ViewOrg => "view_org",
            Self::ViewMembers => "view_members",
            Self::InviteMembers => "invite_members",
            Self::RemoveMembers => "remove_members",
            Self::ChangeRoles => "change_roles",
            Self::TransferOwnership => "transfer_ownership",
            Self::ViewBilling => "view_billing",
            Self::ManageBilling => "manage_billing",
            Self::ManageOrgSettings => "manage_org_settings",
            Self::ManageSso => "manage_sso",
            Self::ManagePolicies => "manage_policies",
            Self::CreateWorkspace => "create_workspace",
            Self::CreateWorkload => "create_workload",
            Self::StartWorkload => "start_workload",
            Self::StopWorkload => "stop_workload",
            Self::DestroyWorkload => "destroy_workload",
            Self::ManageQuotas => "manage_quotas",
            Self::ReadSecrets => "read_secrets",
            Self::WriteSecrets => "write_secrets",
            Self::RunAgentSafe => "run_agent_safe",
            Self::RunAgentDangerous => "run_agent_dangerous",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatStatus {
    Active,
    Invited,
    Suspended,
}

impl SeatStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Invited => "invited",
            Self::Suspended => "suspended",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "invited" => Some(Self::Invited),
            "suspended" => Some(Self::Suspended),
            _ => None,
        }
    }

    pub fn consumes_seat(self) -> bool {
        matches!(self, Self::Active | Self::Invited)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Org {
    pub id: OrgId,
    pub name: String,
    pub slug: String,
    pub seat_limit: u32,
    /// Stripe customer id later; empty for now
    pub billing_customer_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgMember {
    pub org_id: OrgId,
    pub principal_id: PrincipalId,
    pub role: OrgRole,
    pub seat_status: SeatStatus,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgInvitation {
    pub id: OrgInvitationId,
    pub org_id: OrgId,
    pub email: String,
    pub role: OrgRole,
    pub token: String,
    pub invited_by: PrincipalId,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgBillingSnapshot {
    pub org_id: OrgId,
    pub seat_limit: u32,
    pub seats_used: u32,
    pub seats_available: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_is_superset() {
        assert!(OrgRole::Owner.allows(OrgPermission::TransferOwnership));
        assert!(OrgRole::Owner.allows(OrgPermission::DestroyWorkload));
        assert!(OrgRole::Owner.allows(OrgPermission::ManageBilling));
        assert!(!OrgRole::Admin.allows(OrgPermission::TransferOwnership));
        assert!(OrgRole::Admin.allows(OrgPermission::DestroyWorkload));
        assert!(!OrgRole::Member.allows(OrgPermission::DestroyWorkload));
        assert!(OrgRole::Member.allows(OrgPermission::CreateWorkload));
        assert!(OrgRole::Billing.allows(OrgPermission::ViewBilling));
        assert!(!OrgRole::Billing.allows(OrgPermission::InviteMembers));
        assert!(!OrgRole::Member.allows(OrgPermission::RunAgentDangerous));
        assert!(OrgRole::Admin.allows(OrgPermission::RunAgentDangerous));
    }

    #[test]
    fn rank_order() {
        assert!(OrgRole::Owner.at_least(OrgRole::Admin));
        assert!(OrgRole::Admin.at_least(OrgRole::Member));
        assert!(!OrgRole::Member.at_least(OrgRole::Admin));
    }
}

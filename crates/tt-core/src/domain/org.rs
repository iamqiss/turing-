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

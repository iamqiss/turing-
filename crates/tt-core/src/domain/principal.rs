use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::PrincipalId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub id: PrincipalId,
    pub kind: PrincipalKind,
    pub display_name: String,
    pub handle: String,          // unique within a workspace later; globally unique for now
    pub avatar_url: Option<String>,
    pub email: Option<String>,   // humans only
    pub owner_id: Option<PrincipalId>, // agents only – the human who owns this agent
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Principal {
    pub fn is_human(&self) -> bool {
        matches!(self.kind, PrincipalKind::Human)
    }

    pub fn is_agent(&self) -> bool {
        matches!(self.kind, PrincipalKind::Agent)
    }
}

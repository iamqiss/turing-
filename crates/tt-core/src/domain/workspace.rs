use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{PrincipalId, WorkspaceId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub slug: String,            // url-safe unique
    pub description: Option<String>,
    pub owner_id: PrincipalId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
    Guest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMember {
    pub workspace_id: WorkspaceId,
    pub principal_id: PrincipalId,
    pub role: WorkspaceRole,
    pub joined_at: DateTime<Utc>,
}

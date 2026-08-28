use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{PrincipalId, RepositoryId, WorkspaceId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: RepositoryId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub is_private: bool,
    pub created_by: PrincipalId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

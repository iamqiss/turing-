use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ChannelId, PrincipalId, WorkspaceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Public,
    Private,
    Direct,      // 1:1 or small group DM
    Thread,      // reserved if we ever elevate threads
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: ChannelId,
    pub workspace_id: WorkspaceId,
    pub kind: ChannelKind,
    pub name: Option<String>,         // None for pure DMs
    pub topic: Option<String>,
    pub is_archived: bool,
    pub created_by: PrincipalId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMember {
    pub channel_id: ChannelId,
    pub principal_id: PrincipalId,
    pub joined_at: DateTime<Utc>,
    pub last_read_at: Option<DateTime<Utc>>,
}

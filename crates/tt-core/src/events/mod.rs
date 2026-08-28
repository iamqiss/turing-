//! Domain events that flow through the realtime layer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ChannelId, MessageId, PrincipalId, WorkspaceId};

/// Envelope for every realtime event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: uuid::Uuid,
    pub workspace_id: WorkspaceId,
    pub occurred_at: DateTime<Utc>,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    MessageCreated {
        channel_id: ChannelId,
        message_id: MessageId,
        author_id: PrincipalId,
    },
    MessageUpdated {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    MessageDeleted {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    ReactionAdded {
        channel_id: ChannelId,
        message_id: MessageId,
        principal_id: PrincipalId,
        emoji: String,
    },
    ReactionRemoved {
        channel_id: ChannelId,
        message_id: MessageId,
        principal_id: PrincipalId,
        emoji: String,
    },
    ChannelCreated {
        channel_id: ChannelId,
    },
    ChannelMemberJoined {
        channel_id: ChannelId,
        principal_id: PrincipalId,
    },
    ChannelMemberLeft {
        channel_id: ChannelId,
        principal_id: PrincipalId,
    },
    PresenceChanged {
        principal_id: PrincipalId,
        status: PresenceStatus,
    },
    TypingStarted {
        channel_id: ChannelId,
        principal_id: PrincipalId,
    },
    TypingStopped {
        channel_id: ChannelId,
        principal_id: PrincipalId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    Away,
    Offline,
    Busy,
}

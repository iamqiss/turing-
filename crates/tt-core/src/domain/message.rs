use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AttachmentId, ChannelId, MessageId, PrincipalId, ThreadId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub thread_id: Option<ThreadId>,  // if set, this message belongs to a thread
    pub author_id: PrincipalId,
    pub content: String,              // plain text or markdown for v1
    pub reply_to_id: Option<MessageId>,
    pub is_edited: bool,
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub message_id: MessageId,
    pub principal_id: PrincipalId,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub message_id: MessageId,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub storage_key: String,          // object storage key
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub channel_id: ChannelId,
    pub root_message_id: MessageId,
    pub reply_count: u32,
    pub last_reply_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

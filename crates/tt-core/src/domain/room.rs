use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{PrincipalId, RoomId, RoomPostId};

/// Visibility of a Room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomVisibility {
    /// Discoverable community feed — relaxed join, open read for authenticated users
    Public,
    /// Invite / explicit join only
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomPostKind {
    Text,
    VoiceNote,
    VideoClip,
    Attachment,
}

/// Community space (Roomies = members). Not a 1:1 channel — public Rooms are discoverable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: RoomId,
    /// Handle without #, e.g. "rust", "turing-tarpit"
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub visibility: RoomVisibility,
    pub created_by: PrincipalId,
    pub member_count: u32,
    pub is_archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub room_id: RoomId,
    pub principal_id: PrincipalId,
    /// owner | moderator | member
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomPost {
    pub id: RoomPostId,
    pub room_id: RoomId,
    pub author_id: PrincipalId,
    pub kind: RoomPostKind,
    pub content: String,
    /// Optional media object id (voice, clip, file)
    pub media_id: Option<uuid::Uuid>,
    pub is_edited: bool,
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_serde() {
        let v = RoomVisibility::Public;
        let j = serde_json::to_string(&v).unwrap();
        assert!(j.contains("public"));
        let back: RoomVisibility = serde_json::from_str(&j).unwrap();
        assert_eq!(back, RoomVisibility::Public);
    }

    #[test]
    fn post_kind_serde() {
        for k in [
            RoomPostKind::Text,
            RoomPostKind::VoiceNote,
            RoomPostKind::VideoClip,
            RoomPostKind::Attachment,
        ] {
            let j = serde_json::to_string(&k).unwrap();
            let back: RoomPostKind = serde_json::from_str(&j).unwrap();
            assert_eq!(back, k);
        }
    }
}

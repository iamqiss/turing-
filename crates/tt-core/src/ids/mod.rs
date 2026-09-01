//! Strongly-typed identifiers.
//!
//! All IDs are UUID v7 (time-ordered) for better index locality and natural sorting.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            pub fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl AsRef<Uuid> for $name {
            fn as_ref(&self) -> &Uuid {
                &self.0
            }
        }
    };
}

id_type!(WorkspaceId);
id_type!(PrincipalId);
id_type!(ChannelId);
id_type!(MessageId);
id_type!(ThreadId);
id_type!(AttachmentId);
id_type!(RepositoryId);
id_type!(PullRequestId);
id_type!(ReviewId);
id_type!(WorkflowId);
id_type!(AuditEventId);
id_type!(SessionId);
id_type!(IssueId);
id_type!(IssueCommentId);
id_type!(LabelId);
id_type!(OrgId);
id_type!(OrgInvitationId);
id_type!(NodeId);
id_type!(WorkloadId);
id_type!(InstanceId);
id_type!(SubnetId);
id_type!(IpAllocationId);
id_type!(ImageTemplateId);
id_type!(RegionId);
id_type!(RoomId);
id_type!(RoomPostId);

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn room_id_roundtrip_string() {
        let id = RoomId::new();
        let s = id.to_string();
        let parsed = RoomId::from_str(&s).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn ids_are_unique() {
        let a = PrincipalId::new();
        let b = PrincipalId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn from_uuid_preserves() {
        let u = uuid::Uuid::now_v7();
        let id = WorkspaceId::from_uuid(u);
        assert_eq!(*id.as_uuid(), u);
        assert_eq!(id.into_uuid(), u);
    }
}

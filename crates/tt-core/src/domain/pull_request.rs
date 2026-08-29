use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{PrincipalId, PullRequestId, RepositoryId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestStatus {
    Open,
    Closed,
    Merged,
}

impl PullRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            "merged" => Some(Self::Merged),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: PullRequestId,
    pub repository_id: RepositoryId,
    /// Per-repository monotonic number (GitHub-style #42).
    pub number: i32,
    pub title: String,
    pub body: Option<String>,
    pub author_id: PrincipalId,
    pub source_branch: String,
    pub target_branch: String,
    pub status: PullRequestStatus,
    pub merged_by: Option<PrincipalId>,
    pub merged_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

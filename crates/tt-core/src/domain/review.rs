use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{PrincipalId, PullRequestId, ReviewId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Comment,
    Approve,
    RequestChanges,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "comment" => Some(Self::Comment),
            "approve" => Some(Self::Approve),
            "request_changes" => Some(Self::RequestChanges),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: ReviewId,
    pub pull_request_id: PullRequestId,
    pub author_id: PrincipalId,
    pub state: ReviewState,
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
}

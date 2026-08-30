use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{IssueCommentId, IssueId, PrincipalId, RepositoryId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Closed,
}

impl IssueState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: IssueId,
    pub repository_id: RepositoryId,
    pub number: i32,
    pub title: String,
    pub body: Option<String>,
    pub author_id: PrincipalId,
    pub state: IssueState,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueComment {
    pub id: IssueCommentId,
    pub issue_id: IssueId,
    pub author_id: PrincipalId,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

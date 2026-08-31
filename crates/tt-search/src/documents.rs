//! Document shapes indexed into Meilisearch.

use serde::Serialize;
use tt_core::domain::{Issue, PullRequest, Repository};
use tt_core::ids::{PrincipalId, WorkspaceId};

pub const IDX_REPOS: &str = "tt_repos";
pub const IDX_ISSUES: &str = "tt_issues";
pub const IDX_PRS: &str = "tt_pull_requests";
pub const IDX_PEOPLE: &str = "tt_people";
pub const IDX_MESSAGES: &str = "tt_messages";

#[derive(Debug, Serialize)]
pub struct RepoDoc {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub description: String,
    pub is_private: bool,
    pub kind: &'static str,
}

impl RepoDoc {
    pub fn from_repo(r: &Repository, description: Option<&str>) -> Self {
        Self {
            id: r.id.to_string(),
            workspace_id: r.workspace_id.to_string(),
            name: r.name.clone(),
            description: description.map(|s| s.to_string()).or_else(|| r.description.clone()).unwrap_or_default(),
            is_private: r.is_private,
            kind: "repository",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct IssueDoc {
    pub id: String,
    pub repository_id: String,
    pub workspace_id: String,
    pub number: i64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub kind: &'static str,
}

impl IssueDoc {
    pub fn from_issue(i: &Issue, workspace_id: WorkspaceId) -> Self {
        Self {
            id: i.id.to_string(),
            repository_id: i.repository_id.to_string(),
            workspace_id: workspace_id.to_string(),
            number: i.number as i64,
            title: i.title.clone(),
            body: i.body.clone().unwrap_or_default(),
            state: i.state.as_str().to_string(),
            kind: "issue",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PrDoc {
    pub id: String,
    pub repository_id: String,
    pub workspace_id: String,
    pub number: i64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub source_branch: String,
    pub target_branch: String,
    pub kind: &'static str,
}

impl PrDoc {
    pub fn from_pr(p: &PullRequest, workspace_id: WorkspaceId) -> Self {
        Self {
            id: p.id.to_string(),
            repository_id: p.repository_id.to_string(),
            workspace_id: workspace_id.to_string(),
            number: p.number as i64,
            title: p.title.clone(),
            body: p.body.clone().unwrap_or_default(),
            state: p.status.as_str().to_string(),
            source_branch: p.source_branch.clone(),
            target_branch: p.target_branch.clone(),
            kind: "pull_request",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PersonDoc {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub bio: String,
    pub kind: &'static str,
}

impl PersonDoc {
    pub fn new(id: PrincipalId, handle: &str, display_name: &str, bio: Option<&str>) -> Self {
        Self {
            id: id.to_string(),
            handle: handle.to_string(),
            display_name: display_name.to_string(),
            bio: bio.unwrap_or("").to_string(),
            kind: "person",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MessageDoc {
    pub id: String,
    pub channel_id: String,
    pub workspace_id: String,
    pub body: String,
    pub author_id: String,
    pub kind: &'static str,
}


impl MessageDoc {
    pub fn new(
        id: tt_core::ids::MessageId,
        channel_id: tt_core::ids::ChannelId,
        workspace_id: WorkspaceId,
        body: &str,
        author_id: PrincipalId,
    ) -> Self {
        Self {
            id: id.to_string(),
            channel_id: channel_id.to_string(),
            workspace_id: workspace_id.to_string(),
            body: body.to_string(),
            author_id: author_id.to_string(),
            kind: "message",
        }
    }
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{PrincipalId, RepositoryId, WorkspaceId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub principal_id: PrincipalId,
    pub bio: Option<String>,
    pub location: Option<String>,
    pub website: Option<String>,
    /// Free-form links: [{"label":"blog","url":"..."}]
    pub links: serde_json::Value,
    pub is_public: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Push,
    Commit,
    PrOpen,
    PrMerge,
    PrClose,
    Review,
    Message,
    Login,
    AgentRun,
    IssueOpen,
    IssueClose,
    IssueComment,
}

impl ContributionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Commit => "commit",
            Self::PrOpen => "pr_open",
            Self::PrMerge => "pr_merge",
            Self::PrClose => "pr_close",
            Self::Review => "review",
            Self::Message => "message",
            Self::Login => "login",
            Self::AgentRun => "agent_run",
            Self::IssueOpen => "issue_open",
            Self::IssueClose => "issue_close",
            Self::IssueComment => "issue_comment",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "push" => Some(Self::Push),
            "commit" => Some(Self::Commit),
            "pr_open" => Some(Self::PrOpen),
            "pr_merge" => Some(Self::PrMerge),
            "pr_close" => Some(Self::PrClose),
            "review" => Some(Self::Review),
            "message" => Some(Self::Message),
            "login" => Some(Self::Login),
            "agent_run" => Some(Self::AgentRun),
            "issue_open" => Some(Self::IssueOpen),
            "issue_close" => Some(Self::IssueClose),
            "issue_comment" => Some(Self::IssueComment),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    DioxusWeb,
    DioxusDesktop,
    DioxusMobile,
    GitHttp,
    Agent,
    Ci,
    Api,
    Unknown,
}

impl ClientKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DioxusWeb => "dioxus_web",
            Self::DioxusDesktop => "dioxus_desktop",
            Self::DioxusMobile => "dioxus_mobile",
            Self::GitHttp => "git_http",
            Self::Agent => "agent",
            Self::Ci => "ci",
            Self::Api => "api",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dioxus_web" => Some(Self::DioxusWeb),
            "dioxus_desktop" => Some(Self::DioxusDesktop),
            "dioxus_mobile" => Some(Self::DioxusMobile),
            "git_http" => Some(Self::GitHttp),
            "agent" => Some(Self::Agent),
            "ci" => Some(Self::Ci),
            "api" => Some(Self::Api),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributionEvent {
    pub id: uuid::Uuid,
    pub ts: DateTime<Utc>,
    pub principal_id: PrincipalId,
    pub workspace_id: Option<WorkspaceId>,
    pub repository_id: Option<RepositoryId>,
    pub kind: ContributionKind,
    pub client_kind: ClientKind,
    pub tool_id: Option<String>,
    pub languages: Vec<String>,
    pub files_touched: u32,
    pub lines_added: u32,
    pub lines_deleted: u32,
    pub is_automation: bool,
    pub metadata: serde_json::Value,
}

/// Quick maintainer-facing card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileCard {
    pub principal_id: PrincipalId,
    pub handle: String,
    pub display_name: String,
    pub kind: String,
    pub bio: Option<String>,
    pub is_public: bool,
    pub member_since: DateTime<Utc>,
    pub blend: ActivityBlend,
    pub top_languages: Vec<LangWeight>,
    pub top_tools: Vec<ToolWeight>,
    pub density: DensitySummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityBlend {
    pub window_days: u32,
    pub human_interactive_pct: f32,
    pub assisted_agent_pct: f32,
    pub scripted_ci_pct: f32,
    pub thin_activity: bool,
    pub evidence: BlendEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendEvidence {
    pub total_events: u64,
    pub human_events: u64,
    pub agent_events: u64,
    pub ci_events: u64,
    pub thin_commits: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangWeight {
    pub language: String,
    pub weight_pct: f32,
    pub events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolWeight {
    pub client_kind: String,
    pub tool_id: Option<String>,
    pub weight_pct: f32,
    pub events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DensitySummary {
    pub avg_files_per_commit: f32,
    pub avg_lines_changed_per_commit: f32,
    pub commit_events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContribution {
    pub repository_id: RepositoryId,
    pub workspace_id: Option<WorkspaceId>,
    pub events: u64,
    pub last_seen: DateTime<Utc>,
}

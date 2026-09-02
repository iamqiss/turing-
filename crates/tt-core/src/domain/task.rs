use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{OrgId, PrincipalId, RepositoryId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRunStatus {
    Queued,
    InProgress,
    Success,
    Failure,
    Cancelled,
    TimedOut,
}

impl TaskRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "in_progress" => Some(Self::InProgress),
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            "cancelled" => Some(Self::Cancelled),
            "timed_out" => Some(Self::TimedOut),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failure | Self::Cancelled | Self::TimedOut
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTrigger {
    Manual,
    Push,
    PullRequest,
    Schedule,
    WorkflowCall,
}

impl TaskTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Push => "push",
            Self::PullRequest => "pull_request",
            Self::Schedule => "schedule",
            Self::WorkflowCall => "workflow_call",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(Self::Manual),
            "push" => Some(Self::Push),
            "pull_request" => Some(Self::PullRequest),
            "schedule" => Some(Self::Schedule),
            "workflow_call" => Some(Self::WorkflowCall),
            _ => None,
        }
    }
}

/// Pipeline definition discovered under `.tt/pipelines/<name>.yml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPipeline {
    pub id: uuid::Uuid,
    pub repository_id: RepositoryId,
    pub org_id: Option<OrgId>,
    pub name: String,
    /// e.g. `.tt/pipelines/ci.yml`
    pub path: String,
    pub definition_yaml: String,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: uuid::Uuid,
    pub pipeline_id: uuid::Uuid,
    pub repository_id: RepositoryId,
    pub org_id: Option<OrgId>,
    pub pipeline_name: String,
    pub status: TaskRunStatus,
    pub trigger: TaskTrigger,
    pub git_ref: Option<String>,
    pub git_sha: Option<String>,
    pub triggered_by: Option<PrincipalId>,
    pub billable_minutes: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskJob {
    pub id: uuid::Uuid,
    pub run_id: uuid::Uuid,
    pub name: String,
    pub status: TaskRunStatus,
    pub runs_on: String,
    pub billable_minutes: u32,
    pub log_excerpt: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

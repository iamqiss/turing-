use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{OrgId, PrincipalId, WorkspaceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Queued,
    Running,
    AwaitingApproval,
    Succeeded,
    Failed,
    Cancelled,
}

impl AgentRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "awaiting_approval" => Some(Self::AwaitingApproval),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: uuid::Uuid,
    pub agent_id: PrincipalId,
    pub owner_id: PrincipalId,
    pub workspace_id: Option<WorkspaceId>,
    pub org_id: Option<OrgId>,
    pub status: AgentRunStatus,
    pub goal: String,
    pub model: String,
    pub max_steps: u32,
    pub step_count: u32,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentApproval {
    pub id: uuid::Uuid,
    pub run_id: uuid::Uuid,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub status: String,
    pub requested_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<PrincipalId>,
}

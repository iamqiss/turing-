#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
ROOT="${1:-$HOME/tt}"
WF="$ROOT/crates/tt-workflows/src"
DOM="$ROOT/crates/tt-core/src/domain"
mkdir -p "$WF" "$DOM"
rm -rf "$WF/engine" "$WF/definition" "$WF/actions" "$WF/triggers" "$WF/store" 2>/dev/null || true
cat > "$WF/engine.rs" << 'TT_EOF'

TT_EOF
echo "wrote $WF/engine.rs"
cat > "$WF/store.rs" << 'TT_EOF'

TT_EOF
echo "wrote $WF/store.rs"
cat > "$WF/definition.rs" << 'TT_EOF'
//! Parse `.tt/pipelines/*.yml` — GitHub Actions–shaped subset.

use serde::Deserialize;
use std::collections::HashMap;

use crate::error::{Result, TaskError};

#[derive(Debug, Clone, Deserialize)]
pub struct PipelineFile {
    pub name: Option<String>,
    #[serde(default, rename = "on")]
    pub on: Triggers,
    #[serde(default)]
    pub jobs: HashMap<String, JobDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Triggers {
    #[serde(default)]
    pub push: Option<BranchFilter>,
    #[serde(default)]
    pub pull_request: Option<BranchFilter>,
    #[serde(default)]
    pub workflow_dispatch: Option<bool>,
    #[serde(default)]
    pub schedule: Option<Vec<ScheduleItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BranchFilter {
    #[serde(default)]
    pub branches: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleItem {
    pub cron: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobDef {
    #[serde(default = "default_runs_on", rename = "runs-on")]
    pub runs_on: String,
    #[serde(default)]
    pub steps: Vec<StepDef>,
    #[serde(default)]
    pub name: Option<String>,
}

fn default_runs_on() -> String {
    "tt-linux-latest".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepDef {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub uses: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub with: Option<HashMap<String, String>>,
}

pub fn parse_pipeline_yaml(yaml: &str) -> Result<PipelineFile> {
    let p: PipelineFile =
        serde_yaml::from_str(yaml).map_err(|e| TaskError::Invalid(format!("yaml: {e}")))?;
    if p.jobs.is_empty() {
        return Err(TaskError::Invalid("pipeline has no jobs".into()));
    }
    Ok(p)
}

/// Does this pipeline fire for a given trigger + branch?
pub fn matches_trigger(p: &PipelineFile, trigger: &str, branch: Option<&str>) -> bool {
    match trigger {
        "manual" | "workflow_dispatch" => true,
        "push" => match &p.on.push {
            None => false,
            Some(f) if f.branches.is_empty() => true,
            Some(f) => branch
                .map(|b| f.branches.iter().any(|x| x == b || x == "*"))
                .unwrap_or(true),
        },
        "pull_request" => match &p.on.pull_request {
            None => false,
            Some(f) if f.branches.is_empty() => true,
            Some(f) => branch
                .map(|b| f.branches.iter().any(|x| x == b || x == "*"))
                .unwrap_or(true),
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let y = r#"
name: ci
on:
  push:
    branches: [main]
jobs:
  test:
    runs-on: tt-linux-latest
    steps:
      - run: cargo test
"#;
        let p = parse_pipeline_yaml(y).unwrap();
        assert_eq!(p.name.as_deref(), Some("ci"));
        assert!(p.jobs.contains_key("test"));
        assert!(matches_trigger(&p, "push", Some("main")));
        assert!(!matches_trigger(&p, "push", Some("dev")));
    }
}

TT_EOF
echo "wrote $WF/definition.rs"
cat > "$WF/error.rs" << 'TT_EOF'
use thiserror::Error;

pub type Result<T> = std::result::Result<T, TaskError>;

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("not found")]
    NotFound,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("invalid pipeline: {0}")]
    Invalid(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

TT_EOF
echo "wrote $WF/error.rs"
cat > "$WF/actions.rs" << 'TT_EOF'
//! Built-in action kinds (`uses:`) — stubs for marketplace later.

/// Known `uses:` prefixes we understand in v1.
pub fn is_supported_action(uses: &str) -> bool {
    uses.starts_with("tt/") || uses == "checkout" || uses.starts_with("actions/checkout")
}

TT_EOF
echo "wrote $WF/actions.rs"
cat > "$WF/triggers.rs" << 'TT_EOF'
//! Trigger helpers for push / PR / schedule (matching lives in `definition`).

pub use crate::definition::matches_trigger;

TT_EOF
echo "wrote $WF/triggers.rs"
cat > "$WF/lib.rs" << 'TT_EOF'
//! Turing Tasks — CI/CD pipelines defined in `.tt/pipelines/*.yml`.
//!
//! Billed as **task minutes** per month (`tasks.minutes` meter).

pub mod actions;
pub mod definition;
pub mod engine;
pub mod error;
pub mod store;
pub mod triggers;

pub use definition::{matches_trigger, parse_pipeline_yaml, PipelineFile};
pub use engine::{included_minutes, TaskService};
pub use error::{Result, TaskError};
pub use store::{PostgresTaskStore, TaskStore};

// Re-export domain types callers often need
pub use tt_core::domain::{TaskJob, TaskPipeline, TaskRun, TaskRunStatus, TaskTrigger};

TT_EOF
echo "wrote $WF/lib.rs"
cat > "$DOM/task.rs" << 'TT_EOF'
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

TT_EOF
echo "wrote $DOM/task.rs"

# ensure mod task in domain/mod.rs
if ! grep -q 'mod task' "$DOM/mod.rs" 2>/dev/null; then
  echo 'mod task;' >> "$DOM/mod.rs"
  echo 'pub use task::*;' >> "$DOM/mod.rs"
  echo "appended mod task to domain/mod.rs"
fi
# serde_yaml in Cargo.toml
CT="$ROOT/crates/tt-workflows/Cargo.toml"
if ! grep -q serde_yaml "$CT"; then
  echo 'serde_yaml = "0.9"' >> "$CT"
  echo "added serde_yaml dep"
fi
echo "DONE — run: cargo run -p tt-server"

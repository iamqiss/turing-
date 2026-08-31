//! Tool registry — the model never touches the system directly.
//! Small models (OLMo-3-7B) succeed when tools are narrow, typed, and validated.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tt_core::ids::{OrgId, PrincipalId, WorkspaceId};

use crate::error::{AgentError, Result};
use crate::permissions::{DangerLevel, ToolPolicy};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub danger: DangerLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub ok: bool,
    pub data: Value,
    pub summary: String,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn specs(&self) -> Vec<ToolSpec>;
    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolOutput>;
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub owner_id: PrincipalId,
    pub agent_id: PrincipalId,
    pub workspace_id: Option<WorkspaceId>,
    pub org_id: Option<OrgId>,
    pub run_id: uuid::Uuid,
}

#[async_trait]
pub trait SearchTool: Send + Sync {
    async fn search(&self, q: &str, workspace_id: Option<WorkspaceId>, limit: u32) -> Result<Value>;
}

#[async_trait]
pub trait GitTool: Send + Sync {
    async fn list_repos(&self, workspace_id: WorkspaceId) -> Result<Value>;
    async fn get_issue(&self, repository_id: &str, number: i32) -> Result<Value>;
    async fn create_issue(
        &self,
        repository_id: &str,
        title: &str,
        body: Option<&str>,
        author_id: PrincipalId,
    ) -> Result<Value>;
    async fn comment_issue(
        &self,
        repository_id: &str,
        number: i32,
        body: &str,
        author_id: PrincipalId,
    ) -> Result<Value>;
    async fn open_pr(
        &self,
        repository_id: &str,
        title: &str,
        body: Option<&str>,
        source_branch: &str,
        target_branch: &str,
        author_id: PrincipalId,
    ) -> Result<Value>;
}

#[async_trait]
pub trait LifecycleTool: Send + Sync {
    async fn list_events(
        &self,
        org_id: Option<OrgId>,
        workspace_hint: Option<WorkspaceId>,
        limit: u32,
    ) -> Result<Value>;
}

#[async_trait]
pub trait ComputeTool: Send + Sync {
    async fn deploy_from_git(
        &self,
        org_id: OrgId,
        created_by: PrincipalId,
        name: &str,
        repository_id: &str,
        git_ref: &str,
        region: &str,
    ) -> Result<Value>;
}

#[async_trait]
pub trait SecretsTool: Send + Sync {
    /// Store secret; never echo value back.
    async fn put_secret(
        &self,
        org_id: OrgId,
        name: &str,
        value: &str,
        environment: Option<&str>,
        created_by: PrincipalId,
    ) -> Result<Value>;
    /// Pearl-safe: names + kinds only
    async fn list_secret_meta(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        environment: Option<&str>,
    ) -> Result<Value>;
    async fn pearl_probe(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        name: &str,
        environment: Option<&str>,
    ) -> Result<Value>;
}

pub struct BuiltinTools {
    pub search: Option<std::sync::Arc<dyn SearchTool>>,
    pub git: Option<std::sync::Arc<dyn GitTool>>,
    pub lifecycle: Option<std::sync::Arc<dyn LifecycleTool>>,
    pub compute: Option<std::sync::Arc<dyn ComputeTool>>,
    pub secrets: Option<std::sync::Arc<dyn SecretsTool>>,
}

impl BuiltinTools {
    pub fn specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "search".into(),
                description: "Search repos, issues, PRs, messages the user can access.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "limit": { "type": "integer", "default": 8 }
                    },
                    "required": ["query"]
                }),
                danger: DangerLevel::Read,
            },
            ToolSpec {
                name: "list_repos".into(),
                description: "List repositories in the current workspace.".into(),
                parameters: json!({ "type": "object", "properties": {}, "required": [] }),
                danger: DangerLevel::Read,
            },
            ToolSpec {
                name: "get_issue".into(),
                description: "Fetch an issue by repository id and number.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "repository_id": { "type": "string" },
                        "number": { "type": "integer" }
                    },
                    "required": ["repository_id", "number"]
                }),
                danger: DangerLevel::Read,
            },
            ToolSpec {
                name: "create_issue".into(),
                description: "Create an issue. Requires human approval.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "repository_id": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" }
                    },
                    "required": ["repository_id", "title"]
                }),
                danger: DangerLevel::Write,
            },
            ToolSpec {
                name: "comment_issue".into(),
                description: "Comment on an issue. Requires human approval.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "repository_id": { "type": "string" },
                        "number": { "type": "integer" },
                        "body": { "type": "string" }
                    },
                    "required": ["repository_id", "number", "body"]
                }),
                danger: DangerLevel::Write,
            },
            ToolSpec {
                name: "open_pr".into(),
                description: "Open a pull request. Requires human approval.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "repository_id": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                        "source_branch": { "type": "string" },
                        "target_branch": { "type": "string", "default": "main" }
                    },
                    "required": ["repository_id", "title", "source_branch"]
                }),
                danger: DangerLevel::Write,
            },
            ToolSpec {
                name: "lifecycle_events".into(),
                description: "List recent compute/deploy lifecycle events for the org.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "default": 20 }
                    },
                    "required": []
                }),
                danger: DangerLevel::Read,
            },
            ToolSpec {
                name: "deploy_workload".into(),
                description: "Deploy a repository to compute (Firecracker/VM). Always requires human approval.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "repository_id": { "type": "string" },
                        "git_ref": { "type": "string", "default": "main" },
                        "region": { "type": "string", "default": "eu-central" },
                        "org_id": { "type": "string" }
                    },
                    "required": ["name", "repository_id"]
                }),
                danger: DangerLevel::Dangerous,
            },
            ToolSpec {
                name: "scarlet_put_secret".into(),
                description: "Scarlet Letter ✉ — store an encrypted secret for the workspace. Value is never echoed. Requires approval.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "value": { "type": "string" }
                    },
                    "required": ["name", "value"]
                }),
                danger: DangerLevel::Dangerous,
            },
            ToolSpec {
                name: "scarlet_list_secrets".into(),
                description: "List Scarlet Letter secret metadata (names/kinds only; values never returned).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "environment": { "type": "string" } },
                    "required": []
                }),
                danger: DangerLevel::Read,
            },
            ToolSpec {
                name: "scarlet_probe".into(),
                description: "Pearl: check if a secret exists and its value_kind — never returns the value.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "environment": { "type": "string" }
                    },
                    "required": ["name"]
                }),
                danger: DangerLevel::Read,
            },
            ToolSpec {
                name: "finish".into(),
                description: "End the run with a final summary for the user.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "summary": { "type": "string" } },
                    "required": ["summary"]
                }),
                danger: DangerLevel::Read,
            },
        ]
    }

    pub fn policy_for(name: &str) -> ToolPolicy {
        match name {
            "search" | "list_repos" | "get_issue" | "lifecycle_events"
            | "scarlet_list_secrets" | "scarlet_probe" | "finish" => ToolPolicy {
                danger: DangerLevel::Read,
                requires_approval: false,
            },
            "create_issue" | "comment_issue" | "open_pr" => ToolPolicy {
                danger: DangerLevel::Write,
                requires_approval: true,
            },
            "deploy_workload" | "scarlet_put_secret" => ToolPolicy {
                danger: DangerLevel::Dangerous,
                requires_approval: true,
            },
            _ => ToolPolicy {
                danger: DangerLevel::Dangerous,
                requires_approval: true,
            },
        }
    }

    fn require_ws(ctx: &ToolContext) -> Result<WorkspaceId> {
        ctx.workspace_id
            .ok_or_else(|| AgentError::Invalid("workspace_id required on this run".into()))
    }
}

#[async_trait]
impl ToolExecutor for BuiltinTools {
    fn specs(&self) -> Vec<ToolSpec> {
        Self::specs()
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolOutput> {
        match call.name.as_str() {
            "search" => {
                let q = call.arguments.get("query").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("search.query required".into()))?;
                let limit = call.arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
                let search = self.search.as_ref().ok_or_else(|| AgentError::Tool("search not configured".into()))?;
                let data = search.search(q, ctx.workspace_id, limit.min(20)).await?;
                Ok(ToolOutput { ok: true, summary: format!("search: {q}"), data })
            }
            "list_repos" => {
                let ws = Self::require_ws(ctx)?;
                let git = self.git.as_ref().ok_or_else(|| AgentError::Tool("git not configured".into()))?;
                let data = git.list_repos(ws).await?;
                Ok(ToolOutput { ok: true, summary: "listed repositories".into(), data })
            }
            "get_issue" => {
                let rid = call.arguments.get("repository_id").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("repository_id required".into()))?;
                let num = call.arguments.get("number").and_then(|v| v.as_i64())
                    .ok_or_else(|| AgentError::Invalid("number required".into()))? as i32;
                let git = self.git.as_ref().ok_or_else(|| AgentError::Tool("git not configured".into()))?;
                let data = git.get_issue(rid, num).await?;
                Ok(ToolOutput { ok: true, summary: format!("issue #{num}"), data })
            }
            "create_issue" => {
                let rid = call.arguments.get("repository_id").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("repository_id required".into()))?;
                let title = call.arguments.get("title").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("title required".into()))?;
                let body = call.arguments.get("body").and_then(|v| v.as_str());
                let git = self.git.as_ref().ok_or_else(|| AgentError::Tool("git not configured".into()))?;
                let data = git.create_issue(rid, title, body, ctx.owner_id).await?;
                Ok(ToolOutput { ok: true, summary: format!("created issue: {title}"), data })
            }
            "comment_issue" => {
                let rid = call.arguments.get("repository_id").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("repository_id required".into()))?;
                let num = call.arguments.get("number").and_then(|v| v.as_i64())
                    .ok_or_else(|| AgentError::Invalid("number required".into()))? as i32;
                let body = call.arguments.get("body").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("body required".into()))?;
                let git = self.git.as_ref().ok_or_else(|| AgentError::Tool("git not configured".into()))?;
                let data = git.comment_issue(rid, num, body, ctx.owner_id).await?;
                Ok(ToolOutput { ok: true, summary: format!("commented on #{num}"), data })
            }
            "open_pr" => {
                let rid = call.arguments.get("repository_id").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("repository_id required".into()))?;
                let title = call.arguments.get("title").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("title required".into()))?;
                let body = call.arguments.get("body").and_then(|v| v.as_str());
                let source = call.arguments.get("source_branch").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("source_branch required".into()))?;
                let target = call.arguments.get("target_branch").and_then(|v| v.as_str()).unwrap_or("main");
                let git = self.git.as_ref().ok_or_else(|| AgentError::Tool("git not configured".into()))?;
                let data = git.open_pr(rid, title, body, source, target, ctx.owner_id).await?;
                Ok(ToolOutput { ok: true, summary: format!("opened PR: {title}"), data })
            }
            "lifecycle_events" => {
                let limit = call.arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
                let lc = self.lifecycle.as_ref().ok_or_else(|| AgentError::Tool("lifecycle not configured".into()))?;
                let data = lc.list_events(ctx.org_id, ctx.workspace_id, limit.min(50)).await?;
                Ok(ToolOutput { ok: true, summary: "lifecycle events".into(), data })
            }
            "deploy_workload" => {
                let name = call.arguments.get("name").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("name required".into()))?;
                let rid = call.arguments.get("repository_id").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("repository_id required".into()))?;
                let git_ref = call.arguments.get("git_ref").and_then(|v| v.as_str()).unwrap_or("main");
                let region = call.arguments.get("region").and_then(|v| v.as_str()).unwrap_or("eu-central");
                let org = if let Some(oid) = call.arguments.get("org_id").and_then(|v| v.as_str()) {
                    oid.parse().map_err(|_| AgentError::Invalid("bad org_id".into()))?
                } else {
                    ctx.org_id.ok_or_else(|| AgentError::Invalid("org_id required for deploy".into()))?
                };
                let compute = self.compute.as_ref().ok_or_else(|| AgentError::Tool("compute not configured".into()))?;
                let data = compute.deploy_from_git(org, ctx.owner_id, name, rid, git_ref, region).await?;
                Ok(ToolOutput { ok: true, summary: format!("deploy started: {name}"), data })
            }
            "scarlet_put_secret" => {
                let org = ctx.org_id.ok_or_else(|| AgentError::Invalid("org_id required for secrets".into()))?;
                let name = call.arguments.get("name").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("name required".into()))?;
                let value = call.arguments.get("value").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("value required".into()))?;
                let env = call.arguments.get("environment").and_then(|v| v.as_str());
                let secrets = self.secrets.as_ref().ok_or_else(|| AgentError::Tool("secrets not configured".into()))?;
                let data = secrets.put_secret(org, name, value, env, ctx.owner_id).await?;
                Ok(ToolOutput {
                    ok: true,
                    summary: format!("Scarlet Letter stored: {name}"),
                    data,
                })
            }
            "scarlet_list_secrets" => {
                let org = ctx.org_id.ok_or_else(|| AgentError::Invalid("org_id required".into()))?;
                let env = call.arguments.get("environment").and_then(|v| v.as_str());
                let secrets = self.secrets.as_ref().ok_or_else(|| AgentError::Tool("secrets not configured".into()))?;
                let data = secrets.list_secret_meta(org, ctx.owner_id, env).await?;
                Ok(ToolOutput { ok: true, summary: "Scarlet Letter meta (no values)".into(), data })
            }
            "scarlet_probe" => {
                let org = ctx.org_id.ok_or_else(|| AgentError::Invalid("org_id required".into()))?;
                let name = call.arguments.get("name").and_then(|v| v.as_str())
                    .ok_or_else(|| AgentError::Invalid("name required".into()))?;
                let env = call.arguments.get("environment").and_then(|v| v.as_str());
                let secrets = self.secrets.as_ref().ok_or_else(|| AgentError::Tool("secrets not configured".into()))?;
                let data = secrets.pearl_probe(org, ctx.owner_id, name, env).await?;
                Ok(ToolOutput { ok: true, summary: format!("probe {name}"), data })
            }
            "finish" => {
                let summary = call.arguments.get("summary").and_then(|v| v.as_str()).unwrap_or("done");
                Ok(ToolOutput {
                    ok: true,
                    summary: summary.into(),
                    data: json!({ "finished": true, "summary": summary }),
                })
            }
            other => Err(AgentError::Tool(format!("unknown tool: {other}"))),
        }
    }
}

//! Bridge TT subsystems into agent tools (Pearl).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use tt_agents::{
    AgentError, ComputeTool, GitTool, LifecycleTool, Result as AgentResult, SearchTool, SecretsTool,
};
use tt_compute::ComputeService;
use tt_core::domain::{RegionCode, ResourceSpec, SecretValueKind, WorkloadShape};
use tt_core::ids::{OrgId, PrincipalId, RepositoryId, WorkspaceId};
use tt_git::{IssueService, PullRequestService, RepoService};
use tt_search::SearchService;
use tt_secrets::SecretsService;
use tt_storage::LifecycleStore;

pub struct SearchAdapter {
    pub search: Arc<SearchService>,
}

#[async_trait]
impl SearchTool for SearchAdapter {
    async fn search(
        &self,
        q: &str,
        workspace_id: Option<WorkspaceId>,
        limit: u32,
    ) -> AgentResult<Value> {
        let ws: Vec<WorkspaceId> = workspace_id.into_iter().collect();
        self.search
            .search(q, &ws, &[], limit)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))
    }
}

pub struct GitAdapter {
    pub repos: Arc<RepoService>,
    pub issues: Arc<IssueService>,
    pub pull_requests: Arc<PullRequestService>,
}

#[async_trait]
impl GitTool for GitAdapter {
    async fn list_repos(&self, workspace_id: WorkspaceId) -> AgentResult<Value> {
        let list = self
            .repos
            .list_for_workspace(workspace_id)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!(list
            .iter()
            .map(|r| json!({
                "id": r.id.to_string(),
                "name": r.name,
                "description": r.description,
                "is_private": r.is_private,
            }))
            .collect::<Vec<_>>()))
    }

    async fn get_issue(&self, repository_id: &str, number: i32) -> AgentResult<Value> {
        let rid: RepositoryId = repository_id
            .parse()
            .map_err(|_| AgentError::Invalid("bad repository_id".into()))?;
        let issue = self
            .issues
            .get_by_number(rid, number)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({
            "id": issue.id.to_string(),
            "number": issue.number,
            "title": issue.title,
            "body": issue.body,
            "state": issue.state.as_str(),
        }))
    }

    async fn create_issue(
        &self,
        repository_id: &str,
        title: &str,
        body: Option<&str>,
        author_id: PrincipalId,
    ) -> AgentResult<Value> {
        let rid: RepositoryId = repository_id
            .parse()
            .map_err(|_| AgentError::Invalid("bad repository_id".into()))?;
        let issue = self
            .issues
            .create(rid, author_id, title.to_string(), body.map(|s| s.to_string()))
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({
            "id": issue.id.to_string(),
            "number": issue.number,
            "title": issue.title,
        }))
    }

    async fn comment_issue(
        &self,
        repository_id: &str,
        number: i32,
        body: &str,
        author_id: PrincipalId,
    ) -> AgentResult<Value> {
        let rid: RepositoryId = repository_id
            .parse()
            .map_err(|_| AgentError::Invalid("bad repository_id".into()))?;
        let issue = self
            .issues
            .get_by_number(rid, number)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let c = self
            .issues
            .add_comment(issue.id, author_id, body.to_string())
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({
            "comment_id": c.id.to_string(),
            "issue_number": number,
        }))
    }

    async fn open_pr(
        &self,
        repository_id: &str,
        title: &str,
        body: Option<&str>,
        source_branch: &str,
        target_branch: &str,
        author_id: PrincipalId,
    ) -> AgentResult<Value> {
        let rid: RepositoryId = repository_id
            .parse()
            .map_err(|_| AgentError::Invalid("bad repository_id".into()))?;
        let pr = self
            .pull_requests
            .create(
                rid,
                author_id,
                title.to_string(),
                body.map(|s| s.to_string()),
                source_branch.to_string(),
                target_branch.to_string(),
            )
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({
            "id": pr.id.to_string(),
            "number": pr.number,
            "title": pr.title,
            "source_branch": pr.source_branch,
            "target_branch": pr.target_branch,
        }))
    }
}

pub struct LifecycleAdapter {
    pub store: Arc<dyn LifecycleStore>,
}

#[async_trait]
impl LifecycleTool for LifecycleAdapter {
    async fn list_events(
        &self,
        org_id: Option<OrgId>,
        _workspace_hint: Option<WorkspaceId>,
        limit: u32,
    ) -> AgentResult<Value> {
        let since = Utc::now() - Duration::days(14);
        let events = if let Some(org) = org_id {
            self.store
                .list_for_org(org, since, limit)
                .await
                .map_err(|e| AgentError::Tool(e.to_string()))?
        } else {
            self.store
                .list_platform(since, limit)
                .await
                .map_err(|e| AgentError::Tool(e.to_string()))?
        };
        Ok(json!(events
            .iter()
            .map(|e| json!({
                "kind": e.kind.as_str(),
                "ts": e.ts.to_rfc3339(),
                "workload_id": e.workload_id.map(|id| id.to_string()),
                "region": e.region,
                "metadata": e.metadata,
            }))
            .collect::<Vec<_>>()))
    }
}

pub struct ComputeAdapter {
    pub compute: Arc<ComputeService>,
}

#[async_trait]
impl ComputeTool for ComputeAdapter {
    async fn deploy_from_git(
        &self,
        org_id: OrgId,
        created_by: PrincipalId,
        name: &str,
        repository_id: &str,
        git_ref: &str,
        region: &str,
    ) -> AgentResult<Value> {
        let rid: RepositoryId = repository_id
            .parse()
            .map_err(|_| AgentError::Invalid("bad repository_id".into()))?;
        let region = RegionCode::parse(region).unwrap_or(RegionCode::EuCentral);
        let resources = ResourceSpec {
            vcpu: 1,
            memory_mb: 512,
            disk_gb: 5,
        };
        let (wl, inst) = self
            .compute
            .deploy_from_git(
                org_id,
                created_by,
                name.to_string(),
                WorkloadShape::Serverless,
                region,
                resources,
                rid,
                git_ref.to_string(),
                None,
                None,
            )
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({
            "workload_id": wl.id.to_string(),
            "instance_id": inst.id.to_string(),
            "status": wl.status.as_str(),
            "region": wl.region.as_str(),
        }))
    }
}


pub struct SecretsAdapter {
    pub scarlet: Arc<SecretsService>,
}

#[async_trait]
impl SecretsTool for SecretsAdapter {
    async fn put_secret(
        &self,
        org_id: OrgId,
        name: &str,
        value: &str,
        environment: Option<&str>,
        created_by: PrincipalId,
    ) -> AgentResult<Value> {
        let meta = self
            .scarlet
            .put(
                org_id,
                created_by,
                name,
                value,
                environment,
                None,
                SecretValueKind::String,
                None,
                "pearl",
            )
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({
            "name": meta.name,
            "exists": true,
            "version": meta.version,
        }))
    }

    async fn list_secret_meta(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        environment: Option<&str>,
    ) -> AgentResult<Value> {
        let list = self
            .scarlet
            .list_meta(org_id, actor, environment, None, "pearl")
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!(list
            .iter()
            .map(|m| json!({
                "name": m.name,
                "exists": m.exists,
                "value_kind": m.value_kind.as_str(),
                "environment": m.environment,
                "version": m.version,
            }))
            .collect::<Vec<_>>()))
    }

    async fn pearl_probe(
        &self,
        org_id: OrgId,
        actor: PrincipalId,
        name: &str,
        environment: Option<&str>,
    ) -> AgentResult<Value> {
        let m = self
            .scarlet
            .pearl_probe(org_id, actor, name, environment, None)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(json!({
            "name": m.name,
            "exists": m.exists,
            "value_kind": m.value_kind.as_str(),
            "environment": m.environment,
            "version": m.version,
        }))
    }
}

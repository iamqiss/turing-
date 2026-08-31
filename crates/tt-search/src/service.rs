//! Search service — index + multi-index query.

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::{info, warn};
use tt_core::domain::{Issue, PullRequest, Repository};
use tt_core::ids::{PrincipalId, WorkspaceId};

use crate::client::MeiliClient;
use crate::documents::{
    IssueDoc, MessageDoc, PersonDoc, PrDoc, RepoDoc, IDX_ISSUES, IDX_MESSAGES, IDX_PEOPLE, IDX_PRS,
    IDX_REPOS,
};
use crate::error::{Result, SearchError};

#[derive(Clone)]
pub struct SearchService {
    client: Arc<MeiliClient>,
}

impl SearchService {
    pub fn new(client: Arc<MeiliClient>) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &MeiliClient {
        &self.client
    }

    pub fn is_enabled(&self) -> bool {
        self.client.is_enabled()
    }

    /// Create indexes + filterable/searchable attributes.
    pub async fn bootstrap_indexes(&self) {
        if !self.is_enabled() {
            info!("search disabled (Meilisearch not reachable)");
            return;
        }
        self.client
            .soft_ensure(
                IDX_REPOS,
                "id",
                json!({
                    "searchableAttributes": ["name", "description"],
                    "filterableAttributes": ["workspace_id", "is_private", "kind"],
                    "sortableAttributes": ["name"]
                }),
            )
            .await;
        self.client
            .soft_ensure(
                IDX_ISSUES,
                "id",
                json!({
                    "searchableAttributes": ["title", "body"],
                    "filterableAttributes": ["workspace_id", "repository_id", "state", "kind"],
                    "sortableAttributes": ["number"]
                }),
            )
            .await;
        self.client
            .soft_ensure(
                IDX_PRS,
                "id",
                json!({
                    "searchableAttributes": ["title", "body", "source_branch", "target_branch"],
                    "filterableAttributes": ["workspace_id", "repository_id", "state", "kind"],
                    "sortableAttributes": ["number"]
                }),
            )
            .await;
        self.client
            .soft_ensure(
                IDX_PEOPLE,
                "id",
                json!({
                    "searchableAttributes": ["handle", "display_name", "bio"],
                    "filterableAttributes": ["kind"]
                }),
            )
            .await;
        self.client
            .soft_ensure(
                IDX_MESSAGES,
                "id",
                json!({
                    "searchableAttributes": ["body"],
                    "filterableAttributes": ["workspace_id", "channel_id", "kind"]
                }),
            )
            .await;
        info!("meilisearch indexes ensured");
    }

    pub async fn index_repo(&self, repo: &Repository, description: Option<&str>) {
        if !self.is_enabled() {
            return;
        }
        let doc = RepoDoc::from_repo(repo, description);
        if let Ok(v) = serde_json::to_value(&doc) {
            if let Err(e) = self.client.add_documents(IDX_REPOS, &[v]).await {
                warn!(error = %e, repo = %repo.id, "index repo");
            }
        }
    }

    pub async fn index_issue(&self, issue: &Issue, workspace_id: WorkspaceId) {
        if !self.is_enabled() {
            return;
        }
        let doc = IssueDoc::from_issue(issue, workspace_id);
        if let Ok(v) = serde_json::to_value(&doc) {
            if let Err(e) = self.client.add_documents(IDX_ISSUES, &[v]).await {
                warn!(error = %e, issue = %issue.id, "index issue");
            }
        }
    }

    pub async fn index_pr(&self, pr: &PullRequest, workspace_id: WorkspaceId) {
        if !self.is_enabled() {
            return;
        }
        let doc = PrDoc::from_pr(pr, workspace_id);
        if let Ok(v) = serde_json::to_value(&doc) {
            if let Err(e) = self.client.add_documents(IDX_PRS, &[v]).await {
                warn!(error = %e, pr = %pr.id, "index pr");
            }
        }
    }

    pub async fn index_person(
        &self,
        id: PrincipalId,
        handle: &str,
        display_name: &str,
        bio: Option<&str>,
    ) {
        if !self.is_enabled() {
            return;
        }
        let doc = PersonDoc::new(id, handle, display_name, bio);
        if let Ok(v) = serde_json::to_value(&doc) {
            if let Err(e) = self.client.add_documents(IDX_PEOPLE, &[v]).await {
                warn!(error = %e, principal = %id, "index person");
            }
        }
    }

    pub async fn index_message(
        &self,
        id: tt_core::ids::MessageId,
        channel_id: tt_core::ids::ChannelId,
        workspace_id: WorkspaceId,
        body: &str,
        author_id: PrincipalId,
    ) {
        if !self.is_enabled() {
            return;
        }
        if body.trim().is_empty() {
            return;
        }
        let doc = MessageDoc::new(id, channel_id, workspace_id, body, author_id);
        if let Ok(v) = serde_json::to_value(&doc) {
            if let Err(e) = self.client.add_documents(IDX_MESSAGES, &[v]).await {
                warn!(error = %e, message = %id, "index message");
            }
        }
    }

    /// Federated search across indexes. Optional workspace filter for tenant scope.
    pub async fn search(
        &self,
        q: &str,
        workspace_ids: &[WorkspaceId],
        kinds: &[&str],
        limit_per_index: u32,
    ) -> Result<Value> {
        if !self.is_enabled() {
            return Err(SearchError::Disabled);
        }
        let q = q.trim();
        if q.is_empty() {
            return Ok(json!({ "hits": [], "query": "" }));
        }

        let limit = limit_per_index.clamp(1, 50);
        // Empty workspace list → deny workspace-scoped indexes (no cross-tenant leak)
        let ws_filter = if workspace_ids.is_empty() {
            Some("workspace_id = \"__none__\"".to_string())
        } else {
            let parts: Vec<String> = workspace_ids
                .iter()
                .map(|id| format!("workspace_id = \"{}\"", id))
                .collect();
            Some(format!("({})", parts.join(" OR ")))
        };

        let want = |k: &str| kinds.is_empty() || kinds.iter().any(|x| *x == k);

        let mut queries = Vec::new();
        if want("repository") {
            queries.push(json!({
                "indexUid": IDX_REPOS,
                "q": q,
                "limit": limit,
                "filter": ws_filter,
            }));
        }
        if want("issue") {
            queries.push(json!({
                "indexUid": IDX_ISSUES,
                "q": q,
                "limit": limit,
                "filter": ws_filter,
            }));
        }
        if want("pull_request") {
            queries.push(json!({
                "indexUid": IDX_PRS,
                "q": q,
                "limit": limit,
                "filter": ws_filter,
            }));
        }
        if want("person") {
            queries.push(json!({
                "indexUid": IDX_PEOPLE,
                "q": q,
                "limit": limit,
            }));
        }
        if want("message") {
            queries.push(json!({
                "indexUid": IDX_MESSAGES,
                "q": q,
                "limit": limit,
                "filter": ws_filter,
            }));
        }

        // Strip null filters (Meili rejects null)
        for qv in &mut queries {
            if qv.get("filter").map(|f| f.is_null()).unwrap_or(false) {
                qv.as_object_mut().map(|o| o.remove("filter"));
            }
        }

        let body = json!({ "queries": queries });
        let raw = self.client.multi_search(body).await?;

        let mut hits = Vec::new();
        if let Some(results) = raw.get("results").and_then(|r| r.as_array()) {
            for (i, res) in results.iter().enumerate() {
                let index = queries
                    .get(i)
                    .and_then(|q| q.get("indexUid"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("");
                if let Some(arr) = res.get("hits").and_then(|h| h.as_array()) {
                    for h in arr {
                        let mut item = h.clone();
                        if let Some(obj) = item.as_object_mut() {
                            obj.insert("_index".into(), json!(index));
                        }
                        hits.push(item);
                    }
                }
            }
        }

        Ok(json!({
            "query": q,
            "total": hits.len(),
            "hits": hits,
        }))
    }
}

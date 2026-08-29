//! Pull request HTTP API.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tt_auth::extract_bearer;
use tt_core::domain::{MergeMethod, PullRequestStatus};
use tt_core::ids::{PullRequestId, RepositoryId, WorkspaceId};
use tt_git::GitError;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/pulls",
            post(create).get(list),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/pulls/:number",
            get(get_one),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/pulls/:number/close",
            post(close),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/pulls/:number/reopen",
            post(reopen),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/pulls/:number/merge",
            post(merge),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/pulls/:number/status",
            get(status_check),
        )
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub title: String,
    pub body: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct MergeBody {
    /// "merge" | "squash" | "rebase" — defaults to repo setting
    pub method: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct PrJson {
    id: String,
    repository_id: String,
    number: i32,
    title: String,
    body: Option<String>,
    author_id: String,
    source_branch: String,
    target_branch: String,
    status: String,
    merged_by: Option<String>,
    merged_at: Option<String>,
    closed_at: Option<String>,
    created_at: String,
}

fn to_json(pr: &tt_core::domain::PullRequest) -> PrJson {
    PrJson {
        id: pr.id.to_string(),
        repository_id: pr.repository_id.to_string(),
        number: pr.number,
        title: pr.title.clone(),
        body: pr.body.clone(),
        author_id: pr.author_id.to_string(),
        source_branch: pr.source_branch.clone(),
        target_branch: pr.target_branch.clone(),
        status: pr.status.as_str().into(),
        merged_by: pr.merged_by.map(|id| id.to_string()),
        merged_at: pr.merged_at.map(|t| t.to_rfc3339()),
        closed_at: pr.closed_at.map(|t| t.to_rfc3339()),
        created_at: pr.created_at.to_rfc3339(),
    }
}

async fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<tt_auth::AuthContext, StatusCode> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .principal_resolver
        .resolve_bearer(token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

fn map_err(e: GitError) -> axum::response::Response {
    match e {
        GitError::NotFound | GitError::PrNotFound => StatusCode::NOT_FOUND.into_response(),
        GitError::PermissionDenied(m) => (StatusCode::FORBIDDEN, m).into_response(),
        GitError::InvalidArgument(m) | GitError::InvalidName(m) => {
            (StatusCode::BAD_REQUEST, m).into_response()
        }
        GitError::Conflict(m) => (StatusCode::CONFLICT, m).into_response(),
        GitError::AlreadyExists => StatusCode::CONFLICT.into_response(),
        other => {
            tracing::error!(error = %other, "pull request error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

async fn resolve_repo(
    state: &AppState,
    workspace_id: &str,
    repo_name: &str,
) -> Result<(WorkspaceId, RepositoryId), axum::response::Response> {
    let ws: WorkspaceId = workspace_id
        .parse()
        .map_err(|_| StatusCode::BAD_REQUEST.into_response())?;
    let repo = state
        .repos
        .get_by_name(ws, repo_name)
        .await
        .map_err(map_err)?;
    Ok((ws, repo.id))
}

async fn create(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state
        .pull_requests
        .create(
            repo_id,
            ctx.principal_id(),
            body.title,
            body.body,
            body.source_branch,
            body.target_branch,
        )
        .await
    {
        Ok(pr) => (StatusCode::CREATED, Json(to_json(&pr))).into_response(),
        Err(e) => map_err(e),
    }
}

async fn list(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let status = q
        .status
        .as_deref()
        .and_then(PullRequestStatus::parse);
    match state
        .pull_requests
        .list(repo_id, status, q.limit.unwrap_or(50))
        .await
    {
        Ok(list) => Json(list.iter().map(to_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_one(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state.pull_requests.get_by_number(repo_id, number).await {
        Ok(pr) => Json(to_json(&pr)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn close(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let pr = match state.pull_requests.get_by_number(repo_id, number).await {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    match state.pull_requests.close(pr.id, ctx.principal_id()).await {
        Ok(pr) => Json(to_json(&pr)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn reopen(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let pr = match state.pull_requests.get_by_number(repo_id, number).await {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    match state.pull_requests.reopen(pr.id, ctx.principal_id()).await {
        Ok(pr) => Json(to_json(&pr)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn merge(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
    body: Option<Json<MergeBody>>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let pr = match state.pull_requests.get_by_number(repo_id, number).await {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    let requested = body
        .and_then(|b| b.0.method)
        .and_then(|s| MergeMethod::parse(&s));
    match state
        .pull_requests
        .merge(pr.id, ctx.principal_id(), requested)
        .await
    {
        Ok(pr) => Json(to_json(&pr)).into_response(),
        Err(e) => map_err(e),
    }
}


async fn status_check(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let pr = match state.pull_requests.get_by_number(repo_id, number).await {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    let repo = match state.repos.get(repo_id).await {
        Ok(r) => r,
        Err(e) => return map_err(e),
    };
    let path = state.repos.disk_path(repo.workspace_id, &repo.name);
    match tt_git::protect::inspect_branches(&path, &pr.source_branch, &pr.target_branch).await {
        Ok(r) => Json(serde_json::json!({
            "source_sha": r.source_sha,
            "target_sha": r.target_sha,
            "behind_by": r.behind_by,
            "ahead_by": r.ahead_by,
            "is_up_to_date": r.is_up_to_date,
            "has_conflicts": r.has_conflicts,
            "conflict_hint": r.conflict_hint,
            "mergeable": !r.has_conflicts && pr.status.as_str() == "open",
        })).into_response(),
        Err(e) => map_err(e),
    }
}

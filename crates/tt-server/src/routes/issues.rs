//! Issue tracking HTTP API.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tt_auth::extract_bearer;
use tt_core::domain::{ClientKind, ContributionKind, IssueState};
use tt_core::ids::{RepositoryId, WorkspaceId};
use tt_git::GitError;
use tt_storage::events::new_event;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/issues",
            post(create).get(list),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/issues/:number",
            get(get_one).patch(update),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/issues/:number/close",
            post(close),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/issues/:number/reopen",
            post(reopen),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/issues/:number/comments",
            post(add_comment).get(list_comments),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/labels",
            post(create_label).get(list_labels),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/issues/:number/labels",
            post(set_labels).get(get_issue_labels),
        )
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub title: String,
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub state: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CommentBody {
    pub body: String,
}

#[derive(Debug, Serialize)]
struct IssueJson {
    id: String,
    repository_id: String,
    number: i32,
    title: String,
    body: Option<String>,
    author_id: String,
    state: String,
    closed_at: Option<String>,
    created_at: String,
}

fn to_json(i: &tt_core::domain::Issue) -> IssueJson {
    IssueJson {
        id: i.id.to_string(),
        repository_id: i.repository_id.to_string(),
        number: i.number,
        title: i.title.clone(),
        body: i.body.clone(),
        author_id: i.author_id.to_string(),
        state: i.state.as_str().into(),
        closed_at: i.closed_at.map(|t| t.to_rfc3339()),
        created_at: i.created_at.to_rfc3339(),
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
        other => {
            tracing::error!(error = %other, "issue error");
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
        .issues
        .create(repo_id, ctx.principal_id(), body.title, body.body)
        .await
    {
        Ok(issue) => {
            let mut ev = new_event(
                ctx.principal_id(),
                ContributionKind::IssueOpen,
                ClientKind::Api,
            );
            ev.repository_id = Some(repo_id);
            ev.metadata = serde_json::json!({ "number": issue.number, "title": issue.title });
            let _ = state.profiles.events_store().record(&ev).await;
            (StatusCode::CREATED, Json(to_json(&issue))).into_response()
        }
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
    let state_filter = q.state.as_deref().and_then(IssueState::parse);
    match state
        .issues
        .list(repo_id, state_filter, q.limit.unwrap_or(50))
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
    match state.issues.get_by_number(repo_id, number).await {
        Ok(issue) => {
            let strip = state.profiles.card(issue.author_id, 90).await.ok();
            Json(serde_json::json!({
                "issue": to_json(&issue),
                "author_strip": strip,
            }))
            .into_response()
        }
        Err(e) => map_err(e),
    }
}

async fn update(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
    Json(body): Json<UpdateBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let issue = match state.issues.get_by_number(repo_id, number).await {
        Ok(i) => i,
        Err(e) => return map_err(e),
    };
    match state
        .issues
        .update(
            issue.id,
            ctx.principal_id(),
            body.title,
            body.body.map(Some),
        )
        .await
    {
        Ok(i) => Json(to_json(&i)).into_response(),
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
    let issue = match state.issues.get_by_number(repo_id, number).await {
        Ok(i) => i,
        Err(e) => return map_err(e),
    };
    match state.issues.close(issue.id, ctx.principal_id()).await {
        Ok(i) => {
            let mut ev = new_event(
                ctx.principal_id(),
                ContributionKind::IssueClose,
                ClientKind::Api,
            );
            ev.repository_id = Some(repo_id);
            ev.metadata = serde_json::json!({ "number": i.number });
            let _ = state.profiles.events_store().record(&ev).await;
            Json(to_json(&i)).into_response()
        }
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
    let issue = match state.issues.get_by_number(repo_id, number).await {
        Ok(i) => i,
        Err(e) => return map_err(e),
    };
    match state.issues.reopen(issue.id, ctx.principal_id()).await {
        Ok(i) => Json(to_json(&i)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn add_comment(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
    Json(body): Json<CommentBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let issue = match state.issues.get_by_number(repo_id, number).await {
        Ok(i) => i,
        Err(e) => return map_err(e),
    };
    match state
        .issues
        .add_comment(issue.id, ctx.principal_id(), body.body)
        .await
    {
        Ok(c) => {
            let mut ev = new_event(
                ctx.principal_id(),
                ContributionKind::IssueComment,
                ClientKind::Api,
            );
            ev.repository_id = Some(repo_id);
            ev.metadata = serde_json::json!({ "issue": number });
            let _ = state.profiles.events_store().record(&ev).await;
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "id": c.id.to_string(),
                    "issue_id": c.issue_id.to_string(),
                    "author_id": c.author_id.to_string(),
                    "body": c.body,
                    "created_at": c.created_at.to_rfc3339(),
                })),
            )
                .into_response()
        }
        Err(e) => map_err(e),
    }
}

async fn list_comments(
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
    let issue = match state.issues.get_by_number(repo_id, number).await {
        Ok(i) => i,
        Err(e) => return map_err(e),
    };
    match state.issues.list_comments(issue.id, 100).await {
        Ok(list) => Json(
            list.iter()
                .map(|c| {
                    serde_json::json!({
                        "id": c.id.to_string(),
                        "issue_id": c.issue_id.to_string(),
                        "author_id": c.author_id.to_string(),
                        "body": c.body,
                        "created_at": c.created_at.to_rfc3339(),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}


#[derive(Debug, Deserialize)]
pub struct CreateLabelBody {
    pub name: String,
    #[serde(default = "default_color")]
    pub color: String,
    pub description: Option<String>,
}

fn default_color() -> String {
    "888888".into()
}

#[derive(Debug, Deserialize)]
pub struct SetLabelsBody {
    pub labels: Vec<String>,
}

async fn create_label(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<CreateLabelBody>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state
        .issues
        .create_label(repo_id, body.name, body.color, body.description)
        .await
    {
        Ok(l) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": l.id.to_string(),
                "name": l.name,
                "color": l.color,
                "description": l.description,
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_labels(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state.issues.list_labels(repo_id).await {
        Ok(list) => Json(
            list.iter()
                .map(|l| {
                    serde_json::json!({
                        "id": l.id.to_string(),
                        "name": l.name,
                        "color": l.color,
                        "description": l.description,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn set_labels(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
    Json(body): Json<SetLabelsBody>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (_ws, repo_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let issue = match state.issues.get_by_number(repo_id, number).await {
        Ok(i) => i,
        Err(e) => return map_err(e),
    };
    match state.issues.set_issue_labels(issue.id, &body.labels).await {
        Ok(list) => Json(
            list.iter()
                .map(|l| serde_json::json!({ "name": l.name, "color": l.color }))
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_issue_labels(
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
    let issue = match state.issues.get_by_number(repo_id, number).await {
        Ok(i) => i,
        Err(e) => return map_err(e),
    };
    match state.issues.labels_for_issue(issue.id).await {
        Ok(list) => Json(
            list.iter()
                .map(|l| serde_json::json!({ "name": l.name, "color": l.color }))
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

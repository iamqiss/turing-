//! PR review HTTP API.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tt_auth::extract_bearer;
use tt_core::domain::ReviewState;
use tt_core::ids::WorkspaceId;
use tt_git::GitError;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/pulls/:number/reviews",
            post(submit).get(list),
        )
}

#[derive(Debug, Deserialize)]
pub struct SubmitBody {
    /// "comment" | "approve" | "request_changes"
    pub state: String,
    pub body: Option<String>,
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
        GitError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        GitError::Conflict(m) => (StatusCode::CONFLICT, m).into_response(),
        other => {
            tracing::error!(error = %other, "review error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

async fn submit(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
    Json(body): Json<SubmitBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let repo = match state.repos.get_by_name(ws, &repo_name).await {
        Ok(r) => r,
        Err(e) => return map_err(e),
    };
    let pr = match state.pull_requests.get_by_number(repo.id, number).await {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    let Some(st) = ReviewState::parse(&body.state) else {
        return (StatusCode::BAD_REQUEST, "invalid review state").into_response();
    };
    match state
        .reviews
        .submit(pr.id, ctx.principal_id(), st, body.body)
        .await
    {
        Ok(r) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": r.id.to_string(),
                "pull_request_id": r.pull_request_id.to_string(),
                "author_id": r.author_id.to_string(),
                "state": r.state.as_str(),
                "body": r.body,
                "created_at": r.created_at.to_rfc3339(),
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn list(
    State(state): State<AppState>,
    Path((workspace_id, repo_name, number)): Path<(String, String, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let repo = match state.repos.get_by_name(ws, &repo_name).await {
        Ok(r) => r,
        Err(e) => return map_err(e),
    };
    let pr = match state.pull_requests.get_by_number(repo.id, number).await {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    match state.reviews.list(pr.id).await {
        Ok(list) => Json(
            list.iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id.to_string(),
                        "author_id": r.author_id.to_string(),
                        "state": r.state.as_str(),
                        "body": r.body,
                        "created_at": r.created_at.to_rfc3339(),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

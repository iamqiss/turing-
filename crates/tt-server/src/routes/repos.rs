//! Minimal HTTP API for repository management.
//! Auth: Authorization: Bearer <token>

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tt_auth::extract_bearer;
use tt_core::ids::WorkspaceId;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/:workspace_id/repos", post(create_repo).get(list_repos))
        .route(
            "/api/workspaces/:workspace_id/repos/:name",
            get(get_repo).delete(delete_repo),
        )
        .merge(settings_routes())
}

#[derive(Debug, Deserialize)]
pub struct CreateRepoBody {
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub is_private: bool,
    #[serde(default = "default_branch")]
    pub default_branch: String,
}

fn default_true() -> bool {
    true
}
fn default_branch() -> String {
    "main".into()
}

#[derive(Debug, Serialize)]
pub struct RepoResponse {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub is_private: bool,
    pub clone_url_path: String,
}

fn to_response(r: &tt_core::domain::Repository) -> RepoResponse {
    RepoResponse {
        id: r.id.to_string(),
        workspace_id: r.workspace_id.to_string(),
        name: r.name.clone(),
        description: r.description.clone(),
        default_branch: r.default_branch.clone(),
        is_private: r.is_private,
        clone_url_path: format!("/git/{}/{}.git", r.workspace_id, r.name),
    }
}

async fn require_principal(state: &AppState, headers: &HeaderMap) -> Result<tt_auth::AuthContext, StatusCode> {
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

async fn create_repo(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateRepoBody>,
) -> impl IntoResponse {
    let ctx = match require_principal(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    match state
        .repos
        .create(
            ws,
            &body.name,
            body.description,
            body.is_private,
            ctx.principal_id(),
            &body.default_branch,
        )
        .await
    {
        Ok(repo) => (StatusCode::CREATED, Json(to_response(&repo))).into_response(),
        Err(tt_git::GitError::AlreadyExists) => StatusCode::CONFLICT.into_response(),
        Err(tt_git::GitError::InvalidName(m)) => {
            (StatusCode::BAD_REQUEST, m).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "create repo failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_repos(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(s) = require_principal(&state, &headers).await {
        return s.into_response();
    }
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.repos.list_for_workspace(ws).await {
        Ok(list) => {
            let body: Vec<_> = list.iter().map(to_response).collect();
            Json(body).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list repos failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_repo(
    State(state): State<AppState>,
    Path((workspace_id, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(s) = require_principal(&state, &headers).await {
        return s.into_response();
    }
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.repos.get_by_name(ws, &name).await {
        Ok(repo) => Json(to_response(&repo)).into_response(),
        Err(tt_git::GitError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get repo failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_repo(
    State(state): State<AppState>,
    Path((workspace_id, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_principal(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let _ = ctx; // ownership checks later
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.repos.get_by_name(ws, &name).await {
        Ok(repo) => match state.repos.delete(repo.id).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                tracing::error!(error = %e, "delete repo failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Err(tt_git::GitError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get repo failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}


async fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<tt_auth::AuthContext, StatusCode> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(tt_auth::extract_bearer)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .principal_resolver
        .resolve_bearer(token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

// --- Repository settings (merge methods) ---

use tt_core::domain::{MergeMethod, RepositorySettings};
use tt_core::ids::RepositoryId;

pub fn settings_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/:workspace_id/repos/:name/settings",
            get(get_settings).patch(update_settings),
        )
}

#[derive(Debug, Serialize)]
struct SettingsJson {
    repository_id: String,
    allowed_merge_methods: Vec<String>,
    default_merge_method: String,
    delete_branch_on_merge: bool,
    require_up_to_date: bool,
    restrict_merge_to_admins: bool,
    required_approvals: u32,
}

fn settings_json(s: &RepositorySettings) -> SettingsJson {
    SettingsJson {
        repository_id: s.repository_id.to_string(),
        allowed_merge_methods: s
            .allowed_merge_methods
            .iter()
            .map(|m| m.as_str().to_string())
            .collect(),
        default_merge_method: s.default_merge_method.as_str().into(),
        delete_branch_on_merge: s.delete_branch_on_merge,
        require_up_to_date: s.require_up_to_date,
        restrict_merge_to_admins: s.restrict_merge_to_admins,
        required_approvals: s.required_approvals,
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsBody {
    pub allowed_merge_methods: Option<Vec<String>>,
    pub default_merge_method: Option<String>,
    pub delete_branch_on_merge: Option<bool>,
    pub require_up_to_date: Option<bool>,
    pub restrict_merge_to_admins: Option<bool>,
    pub required_approvals: Option<u32>,
}

async fn get_settings(
    State(state): State<AppState>,
    Path((workspace_id, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let repo = match state.repos.get_by_name(ws, &name).await {
        Ok(r) => r,
        Err(e) => return map_git_err(e),
    };
    match state.repos.get_settings(repo.id).await {
        Ok(s) => Json(settings_json(&s)).into_response(),
        Err(e) => map_git_err(e),
    }
}

async fn update_settings(
    State(state): State<AppState>,
    Path((workspace_id, name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpdateSettingsBody>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let repo = match state.repos.get_by_name(ws, &name).await {
        Ok(r) => r,
        Err(e) => return map_git_err(e),
    };
    let mut s = match state.repos.get_settings(repo.id).await {
        Ok(s) => s,
        Err(e) => return map_git_err(e),
    };
    if let Some(methods) = body.allowed_merge_methods {
        let parsed: Result<Vec<_>, _> = methods
            .iter()
            .map(|m| {
                MergeMethod::parse(m).ok_or_else(|| format!("invalid merge method: {m}"))
            })
            .collect();
        match parsed {
            Ok(m) if m.is_empty() => {
                return (StatusCode::BAD_REQUEST, "allowed_merge_methods empty").into_response();
            }
            Ok(m) => s.allowed_merge_methods = m,
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        }
    }
    if let Some(d) = body.default_merge_method {
        match MergeMethod::parse(&d) {
            Some(m) => s.default_merge_method = m,
            None => return (StatusCode::BAD_REQUEST, "invalid default_merge_method").into_response(),
        }
    }
    if let Some(b) = body.delete_branch_on_merge {
        s.delete_branch_on_merge = b;
    }
    match state.repos.update_settings(s).await {
        Ok(s) => Json(settings_json(&s)).into_response(),
        Err(e) => map_git_err(e),
    }
}

fn map_git_err(e: tt_git::GitError) -> axum::response::Response {
    match e {
        tt_git::GitError::NotFound => StatusCode::NOT_FOUND.into_response(),
        tt_git::GitError::InvalidArgument(m) | tt_git::GitError::InvalidName(m) => {
            (StatusCode::BAD_REQUEST, m).into_response()
        }
        tt_git::GitError::PermissionDenied(m) => (StatusCode::FORBIDDEN, m).into_response(),
        other => {
            tracing::error!(error = %other, "repo settings error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

//! Workspace HTTP API.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tt_auth::extract_bearer;
use tt_core::ids::WorkspaceId;
use tt_messaging::MessagingError;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces", post(create).get(list))
        .route("/api/workspaces/:id", get(get_one))
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceJson {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    owner_id: String,
}

fn to_json(w: &tt_core::domain::Workspace) -> WorkspaceJson {
    WorkspaceJson {
        id: w.id.to_string(),
        name: w.name.clone(),
        slug: w.slug.clone(),
        description: w.description.clone(),
        owner_id: w.owner_id.to_string(),
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

fn map_err(e: MessagingError) -> axum::response::Response {
    match e {
        MessagingError::NotFound => StatusCode::NOT_FOUND.into_response(),
        MessagingError::AlreadyExists => StatusCode::CONFLICT.into_response(),
        MessagingError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        MessagingError::PermissionDenied(m) => (StatusCode::FORBIDDEN, m).into_response(),
        MessagingError::Other(err) => {
            tracing::error!(error = %err, "workspace error");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state
        .workspaces
        .create(body.name, body.slug, body.description, ctx.principal_id())
        .await
    {
        Ok(ws) => (StatusCode::CREATED, Json(to_json(&ws))).into_response(),
        Err(e) => map_err(e),
    }
}

async fn list(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.workspaces.list_for_principal(ctx.principal_id()).await {
        Ok(list) => Json(list.iter().map(to_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let ws_id: WorkspaceId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if let Err(e) = state.workspaces.ensure_member(ws_id, ctx.principal_id()).await {
        return map_err(e);
    }
    match state.workspaces.get(ws_id).await {
        Ok(ws) => Json(to_json(&ws)).into_response(),
        Err(e) => map_err(e),
    }
}

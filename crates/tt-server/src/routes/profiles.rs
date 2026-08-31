//! Principal profile + activity blend API.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get};
use axum::{Json, Router};
use serde::{Deserialize};
use tt_auth::extract_bearer;
use tt_core::ids::PrincipalId;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/principals/:id/profile", get(get_card).patch(update_profile))
        .route("/api/principals/:id/activity-blend", get(get_blend))
        .route("/api/principals/:id/repos", get(get_repos))
        .route("/api/me/profile", get(me_card).patch(me_update))
}

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    pub bio: Option<String>,
    pub location: Option<String>,
    pub website: Option<String>,
    pub links: Option<serde_json::Value>,
    pub is_public: Option<bool>,
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

fn map_err(e: tt_storage::StorageError) -> axum::response::Response {
    match e {
        tt_storage::StorageError::NotFound => StatusCode::NOT_FOUND.into_response(),
        tt_storage::StorageError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        other => {
            tracing::error!(error = %other, "profile error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

async fn get_card(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<WindowQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let _ = require_auth(&state, &headers).await; // allow public later; require auth for now
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let pid: PrincipalId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.profiles.card(pid, q.days.unwrap_or(90)).await {
        Ok(card) => Json(card).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_blend(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<WindowQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let pid: PrincipalId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.profiles.card(pid, q.days.unwrap_or(90)).await {
        Ok(card) => Json(card.blend).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_repos(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<WindowQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let pid: PrincipalId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.profiles.repos(pid, q.days.unwrap_or(90)).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => map_err(e),
    }
}

async fn update_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpdateBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let pid: PrincipalId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if ctx.principal_id() != pid {
        return StatusCode::FORBIDDEN.into_response();
    }
    match state
        .profiles
        .update(
            pid,
            body.bio.map(Some),
            body.location.map(Some),
            body.website.map(Some),
            body.links,
            body.is_public,
        )
        .await
    {
        Ok(p) => Json(p).into_response(),
        Err(e) => map_err(e),
    }
}

async fn me_card(
    State(state): State<AppState>,
    Query(q): Query<WindowQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.profiles.card(ctx.principal_id(), q.days.unwrap_or(90)).await {
        Ok(card) => Json(card).into_response(),
        Err(e) => map_err(e),
    }
}

async fn me_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state
        .profiles
        .update(
            ctx.principal_id(),
            body.bio.map(Some),
            body.location.map(Some),
            body.website.map(Some),
            body.links,
            body.is_public,
        )
        .await
    {
        Ok(p) => Json(p).into_response(),
        Err(e) => map_err(e),
    }
}

//! Federated search API + reindex.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tt_core::ids::WorkspaceId;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/search", get(search))
        .route("/api/search/reindex", post(reindex))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    /// Comma-separated: repository,issue,pull_request,person,message
    pub kinds: Option<String>,
    /// Optional workspace scope (intersected with membership)
    pub workspace_ids: Option<String>,
    pub limit: Option<u32>,
}

async fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<tt_auth::AuthContext, StatusCode> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.trim()))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .principal_resolver
        .resolve_bearer(token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

async fn search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if !state.search.is_enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "search_unavailable",
                "message": "Meilisearch is not configured or not reachable"
            })),
        )
            .into_response();
    }

    // Tenant filter: only workspaces the caller belongs to
    let member_ws = match state.workspaces.list_for_principal(ctx.principal_id()).await {
        Ok(list) => list.into_iter().map(|w| w.id).collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    let member_set: std::collections::HashSet<_> = member_ws.iter().copied().collect();

    let requested: Vec<WorkspaceId> = q
        .workspace_ids
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            s.parse().ok()
        })
        .collect();

    let workspace_ids: Vec<WorkspaceId> = if requested.is_empty() {
        member_ws
    } else {
        requested
            .into_iter()
            .filter(|id| member_set.contains(id))
            .collect()
    };

    // No membership → no workspace-scoped hits (people still allowed via kinds)
    let kinds: Vec<&str> = q
        .kinds
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let kinds_ref: Vec<&str> = if kinds.is_empty() {
        vec![]
    } else {
        kinds
    };

    // If caller has zero workspaces, only person search is meaningful
    let limit = q.limit.unwrap_or(10).min(50);
    match state
        .search
        .search(&q.q, &workspace_ids, &kinds_ref, limit)
        .await
    {
        Ok(body) => Json(body).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "search failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

/// Platform operator or any authed user in lab — bulk reindex from Postgres.
async fn reindex(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !state.search.is_enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "search_unavailable" })),
        )
            .into_response();
    }
    match state.search.reindex_from_postgres(&state.db).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

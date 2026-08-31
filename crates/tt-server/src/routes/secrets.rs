//! Scarlet Letter HTTP API.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tt_core::domain::SecretValueKind;
use tt_core::ids::{OrgId, RepositoryId};
use tt_secrets::{scan_text, SecretsError};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/orgs/:slug/secrets",
            get(list_meta).post(put_secret),
        )
        .route(
            "/api/orgs/:slug/secrets/:name",
            get(pearl_probe).delete(delete_secret),
        )
        .route("/api/orgs/:slug/secrets/pull", post(pull_map))
        .route("/api/secrets/scan", post(scan_content))
        .route("/api/secrets/pre-commit-hook", get(pre_commit_hook))
}

#[derive(Debug, Deserialize)]
pub struct ScopeQuery {
    pub environment: Option<String>,
    pub repository_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PutBody {
    pub name: String,
    pub value: String,
    pub environment: Option<String>,
    pub repository_id: Option<String>,
    pub value_kind: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PullBody {
    pub names: Vec<String>,
    pub environment: Option<String>,
    pub repository_id: Option<String>,
    /// cli | ide | ci | api
    pub client_kind: Option<String>,
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

async fn require_org_member(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
) -> Result<(tt_auth::AuthContext, tt_core::domain::Org), StatusCode> {
    let ctx = require_auth(state, headers).await?;
    let org = state
        .orgs
        .get_by_slug(slug)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    state
        .orgs
        .require_active_member(org.id, ctx.principal_id())
        .await
        .map_err(|_| StatusCode::FORBIDDEN)?;
    Ok((ctx, org))
}

fn map_err(e: SecretsError) -> axum::response::Response {
    match e {
        SecretsError::NotFound => StatusCode::NOT_FOUND.into_response(),
        SecretsError::Forbidden(m) => (StatusCode::FORBIDDEN, m).into_response(),
        SecretsError::Invalid(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        other => {
            tracing::error!(error = %other, "scarlet error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

fn meta_json(m: &tt_core::domain::SecretMeta) -> serde_json::Value {
    serde_json::json!({
        "id": m.id.to_string(),
        "name": m.name,
        "exists": m.exists,
        "scope": match m.scope {
            tt_core::domain::SecretScopeKind::Org => "org",
            tt_core::domain::SecretScopeKind::Environment => "environment",
            tt_core::domain::SecretScopeKind::Repository => "repository",
        },
        "environment": m.environment,
        "repository_id": m.repository_id.map(|id| id.to_string()),
        "value_kind": m.value_kind.as_str(),
        "description": m.description,
        "version": m.version,
        "last_rotated_at": m.last_rotated_at.map(|t| t.to_rfc3339()),
        "updated_at": m.updated_at.to_rfc3339(),
    })
}

async fn list_meta(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<ScopeQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let (ctx, org) = match require_org_member(&state, &headers, &slug).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    let rid = q
        .repository_id
        .as_deref()
        .and_then(|s| s.parse::<RepositoryId>().ok());
    match state
        .scarlet
        .list_meta(
            org.id,
            ctx.principal_id(),
            q.environment.as_deref(),
            rid,
            "api",
        )
        .await
    {
        Ok(list) => Json(list.iter().map(meta_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

async fn put_secret(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PutBody>,
) -> impl IntoResponse {
    let (ctx, org) = match require_org_member(&state, &headers, &slug).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    // Writes: adminish
    if state
        .orgs
        .require_admin(org.id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let rid = body
        .repository_id
        .as_deref()
        .and_then(|s| s.parse::<RepositoryId>().ok());
    let kind = body
        .value_kind
        .as_deref()
        .and_then(SecretValueKind::parse)
        .unwrap_or(SecretValueKind::String);
    match state
        .scarlet
        .put(
            org.id,
            ctx.principal_id(),
            &body.name,
            &body.value,
            body.environment.as_deref(),
            rid,
            kind,
            body.description,
            "api",
        )
        .await
    {
        Ok(m) => (StatusCode::CREATED, Json(meta_json(&m))).into_response(),
        Err(e) => map_err(e),
    }
}

async fn pearl_probe(
    State(state): State<AppState>,
    Path((slug, name)): Path<(String, String)>,
    Query(q): Query<ScopeQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let (ctx, org) = match require_org_member(&state, &headers, &slug).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    let rid = q
        .repository_id
        .as_deref()
        .and_then(|s| s.parse::<RepositoryId>().ok());
    match state
        .scarlet
        .pearl_probe(
            org.id,
            ctx.principal_id(),
            &name,
            q.environment.as_deref(),
            rid,
        )
        .await
    {
        Ok(m) => Json(meta_json(&m)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn delete_secret(
    State(state): State<AppState>,
    Path((slug, name)): Path<(String, String)>,
    Query(q): Query<ScopeQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let (ctx, org) = match require_org_member(&state, &headers, &slug).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    if state
        .orgs
        .require_admin(org.id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let rid = q
        .repository_id
        .as_deref()
        .and_then(|s| s.parse::<RepositoryId>().ok());
    match state
        .scarlet
        .delete(
            org.id,
            ctx.principal_id(),
            &name,
            q.environment.as_deref(),
            rid,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

async fn pull_map(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PullBody>,
) -> impl IntoResponse {
    let (ctx, org) = match require_org_member(&state, &headers, &slug).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    let rid = body
        .repository_id
        .as_deref()
        .and_then(|s| s.parse::<RepositoryId>().ok());
    let client = body.client_kind.as_deref().unwrap_or("cli");
    match state
        .scarlet
        .pull_map(
            org.id,
            ctx.principal_id(),
            &body.names,
            body.environment.as_deref(),
            rid,
            client,
        )
        .await
    {
        // Values only leave the server for authenticated pull clients (CLI/CI).
        // Never log this response body.
        Ok(map) => Json(serde_json::json!({ "secrets": map })).into_response(),
        Err(e) => map_err(e),
    }
}


#[derive(Debug, Deserialize)]
pub struct ScanBody {
    pub content: String,
    pub allowlist: Option<Vec<String>>,
}

async fn scan_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ScanBody>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut report = scan_text(&body.content);
    if let Some(ref allow) = body.allowlist {
        report = tt_secrets::apply_allowlist(report, allow);
    }
    Json(report).into_response()
}

async fn pre_commit_hook() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        tt_git::pre_commit_hook_script(),
    )
}

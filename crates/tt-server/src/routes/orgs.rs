//! Organization HTTP API (seats, invites, policies, SSO, billing link).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use tt_auth::extract_bearer;
use tt_core::domain::OrgRole;
use tt_messaging::MessagingError;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/orgs", post(create).get(list_mine))
        .route("/api/orgs/:slug", get(get_one))
        .route("/api/orgs/:slug/billing", get(billing))
        .route("/api/orgs/:slug/members", get(list_members))
        .route(
            "/api/orgs/:slug/invitations",
            post(invite).get(list_invitations),
        )
        .route("/api/orgs/invitations/:token/accept", post(accept_invite))
        .route("/api/orgs/:slug/seat-limit", post(set_seat_limit))
        .route("/api/orgs/:slug/workspaces", post(create_workspace))
        .route(
            "/api/orgs/:slug/settings",
            get(get_policy).patch(update_policy),
        )
        .route("/api/orgs/:slug/sso", get(get_sso).put(put_sso))
        .route("/api/orgs/:slug/billing/link", post(link_stripe))
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub name: String,
    pub slug: String,
    #[serde(default = "default_seats")]
    pub seat_limit: u32,
}

fn default_seats() -> u32 {
    5
}

#[derive(Debug, Deserialize)]
pub struct InviteBody {
    pub email: String,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "member".into()
}

#[derive(Debug, Deserialize)]
pub struct SeatLimitBody {
    pub seat_limit: u32,
}

#[derive(Debug, Deserialize)]
pub struct CreateWsBody {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PolicyBody {
    pub allowed_merge_methods: Option<Vec<String>>,
    pub default_merge_method: Option<String>,
    pub delete_branch_on_merge: Option<bool>,
    pub require_up_to_date: Option<bool>,
    pub restrict_merge_to_admins: Option<bool>,
    pub required_approvals: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct SsoBody {
    pub enabled: bool,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub domains: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct LinkStripeBody {
    pub customer_id: String,
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
        MessagingError::Conflict(m) => (StatusCode::CONFLICT, m).into_response(),
        MessagingError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        MessagingError::PermissionDenied(m) | MessagingError::Forbidden(m) => {
            (StatusCode::FORBIDDEN, m).into_response()
        }
        other => {
            tracing::error!(error = %other, "org error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

fn org_json(o: &tt_core::domain::Org) -> serde_json::Value {
    serde_json::json!({
        "id": o.id.to_string(),
        "name": o.name,
        "slug": o.slug,
        "seat_limit": o.seat_limit,
        "created_at": o.created_at.to_rfc3339(),
    })
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
        .orgs
        .create(body.name, body.slug, ctx.principal_id(), body.seat_limit)
        .await
    {
        Ok(o) => (StatusCode::CREATED, Json(org_json(&o))).into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_mine(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.orgs.list_mine(ctx.principal_id()).await {
        Ok(list) => Json(list.iter().map(org_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_one(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.orgs.get_by_slug(&slug).await {
        Ok(o) => Json(org_json(&o)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn billing(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    match state.orgs.billing(org.id).await {
        Ok(b) => Json(serde_json::json!({
            "org_id": b.org_id.to_string(),
            "seat_limit": b.seat_limit,
            "seats_used": b.seats_used,
            "seats_available": b.seats_available,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_members(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    match state.orgs.list_members(org.id).await {
        Ok(list) => Json(
            list.iter()
                .map(|m| {
                    serde_json::json!({
                        "principal_id": m.principal_id.to_string(),
                        "role": m.role.as_str(),
                        "seat_status": m.seat_status.as_str(),
                        "joined_at": m.joined_at.to_rfc3339(),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn invite(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<InviteBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    let Some(role) = OrgRole::parse(&body.role) else {
        return (StatusCode::BAD_REQUEST, "invalid role").into_response();
    };
    match state
        .orgs
        .invite(org.id, ctx.principal_id(), body.email, role)
        .await
    {
        Ok(inv) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": inv.id.to_string(),
                "email": inv.email,
                "role": inv.role.as_str(),
                "token": inv.token,
                "expires_at": inv.expires_at.to_rfc3339(),
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_invitations(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    if let Err(e) = state.orgs.require_admin(org.id, ctx.principal_id()).await {
        return map_err(e);
    }
    match state.orgs.list_invitations(org.id).await {
        Ok(list) => Json(
            list.iter()
                .map(|i| {
                    serde_json::json!({
                        "id": i.id.to_string(),
                        "email": i.email,
                        "role": i.role.as_str(),
                        "expires_at": i.expires_at.to_rfc3339(),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn accept_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state
        .orgs
        .accept_invitation(&token, ctx.principal_id())
        .await
    {
        Ok(m) => Json(serde_json::json!({
            "org_id": m.org_id.to_string(),
            "role": m.role.as_str(),
            "seat_status": m.seat_status.as_str(),
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn set_seat_limit(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SeatLimitBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    match state
        .orgs
        .set_seat_limit(org.id, ctx.principal_id(), body.seat_limit)
        .await
    {
        Ok(o) => Json(org_json(&o)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn create_workspace(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateWsBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    if let Err(e) = state.orgs.require_admin(org.id, ctx.principal_id()).await {
        return map_err(e);
    }
    match state
        .workspaces
        .create_in_org(
            body.name,
            body.slug,
            body.description,
            ctx.principal_id(),
            org.id,
        )
        .await
    {
        Ok(ws) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": ws.id.to_string(),
                "org_id": ws.org_id.map(|id| id.to_string()),
                "name": ws.name,
                "slug": ws.slug,
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_policy(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    match state.orgs.get_policy(org.id).await {
        Ok(p) => Json(serde_json::json!({
            "allowed_merge_methods": p.allowed_merge_methods.iter().map(|m| m.as_str()).collect::<Vec<_>>(),
            "default_merge_method": p.default_merge_method.as_str(),
            "delete_branch_on_merge": p.delete_branch_on_merge,
            "require_up_to_date": p.require_up_to_date,
            "restrict_merge_to_admins": p.restrict_merge_to_admins,
            "required_approvals": p.required_approvals,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn update_policy(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PolicyBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    if let Err(e) = state.orgs.require_admin(org.id, ctx.principal_id()).await {
        return map_err(e);
    }
    let mut p = match state.orgs.get_policy(org.id).await {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    if let Some(methods) = body.allowed_merge_methods {
        let parsed: Result<Vec<_>, _> = methods
            .iter()
            .map(|m| tt_core::domain::MergeMethod::parse(m).ok_or(m.as_str()))
            .collect();
        match parsed {
            Ok(m) if !m.is_empty() => p.allowed_merge_methods = m,
            Ok(_) => {
                return (StatusCode::BAD_REQUEST, "allowed_merge_methods empty").into_response();
            }
            Err(m) => {
                return (StatusCode::BAD_REQUEST, format!("invalid method {m}")).into_response();
            }
        }
    }
    if let Some(d) = body.default_merge_method {
        match tt_core::domain::MergeMethod::parse(&d) {
            Some(m) => p.default_merge_method = m,
            None => {
                return (StatusCode::BAD_REQUEST, "invalid default_merge_method").into_response();
            }
        }
    }
    if let Some(b) = body.delete_branch_on_merge {
        p.delete_branch_on_merge = b;
    }
    if let Some(b) = body.require_up_to_date {
        p.require_up_to_date = b;
    }
    if let Some(b) = body.restrict_merge_to_admins {
        p.restrict_merge_to_admins = b;
    }
    if let Some(n) = body.required_approvals {
        p.required_approvals = n;
    }
    if let Err(e) = state.orgs.upsert_policy(&p).await {
        return map_err(e);
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn get_sso(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    if let Err(e) = state.orgs.require_admin(org.id, ctx.principal_id()).await {
        return map_err(e);
    }
    match state.orgs.get_sso(org.id).await {
        Ok(s) => Json(serde_json::json!({
            "enabled": s.enabled,
            "provider": s.provider,
            "issuer_url": s.issuer_url,
            "client_id": s.client_id,
            "client_secret_set": s.client_secret_set,
            "domains": s.domains,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn put_sso(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SsoBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    if let Err(e) = state.orgs.require_admin(org.id, ctx.principal_id()).await {
        return map_err(e);
    }
    match state
        .orgs
        .upsert_sso(
            org.id,
            body.enabled,
            body.issuer_url,
            body.client_id,
            body.client_secret,
            body.domains.unwrap_or_default(),
        )
        .await
    {
        Ok(s) => Json(serde_json::json!({
            "enabled": s.enabled,
            "issuer_url": s.issuer_url,
            "client_id": s.client_id,
            "client_secret_set": s.client_secret_set,
            "domains": s.domains,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn link_stripe(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<LinkStripeBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => return map_err(e),
    };
    if let Err(e) = state.orgs.require_admin(org.id, ctx.principal_id()).await {
        return map_err(e);
    }
    match state
        .orgs
        .link_stripe_customer(org.id, &body.customer_id)
        .await
    {
        Ok(()) => Json(serde_json::json!({
            "linked": true,
            "customer_id": body.customer_id
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

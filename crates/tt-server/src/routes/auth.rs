//! Passphrase + session HTTP API (CLI-friendly).
//!
//! WebAuthn remains on gRPC; passphrases are available on both REST and gRPC.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::extract::Path;
use axum::routing::{delete, get, post, put};
use uuid::Uuid;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tt_auth::{extract_bearer, AuthError};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/passphrase", put(set_passphrase))
        // Break Glass
        .route("/api/auth/break-glass/settings", get(bg_get_settings).put(bg_put_settings))
        .route("/api/auth/break-glass/trustees", get(bg_list_trustees).post(bg_add_trustee))
        .route("/api/auth/break-glass/trustees/:id", delete(bg_remove_trustee))
        .route("/api/auth/break-glass/time-lock", post(bg_trigger_time_lock))
        .route("/api/auth/break-glass/time-lock/:id/cancel", post(bg_cancel_time_lock))
        .route("/api/auth/break-glass/trustees/recover", post(bg_trigger_trustees))
        .route("/api/auth/break-glass/events/:id/approve", post(bg_approve))
        .route("/api/auth/break-glass/events/:id", get(bg_get_event))
        .route("/api/auth/break-glass/events/:id/claim", post(bg_claim))
        .route("/api/auth/break-glass/events/:id/complete", post(bg_complete))
}

#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub passphrase: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub handle: String,
    pub passphrase: String,
}

#[derive(Debug, Deserialize)]
pub struct SetPassphraseBody {
    pub passphrase: String,
}

#[derive(Debug, Serialize)]
struct SessionResponse {
    principal_id: String,
    handle: String,
    display_name: String,
    session_token: String,
}

fn map_err(e: AuthError) -> axum::response::Response {
    use AuthError::*;
    match e {
        HandleTaken | EmailTaken => (StatusCode::CONFLICT, e.to_string()).into_response(),
        Unauthenticated | InvalidToken | SessionNotFound | TokenExpired => {
            (StatusCode::UNAUTHORIZED, e.to_string()).into_response()
        }
        InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        PrincipalInactive | PermissionDenied(_) => {
            (StatusCode::FORBIDDEN, e.to_string()).into_response()
        }
        PrincipalNotFound => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
        other => {
            tracing::error!(error = %other, "auth error");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
    }
}

async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> impl IntoResponse {
    match state
        .passphrases
        .register(
            &body.handle,
            &body.display_name,
            body.email.as_deref(),
            &body.passphrase,
        )
        .await
    {
        Ok((p, token)) => (
            StatusCode::CREATED,
            Json(SessionResponse {
                principal_id: p.id.to_string(),
                handle: p.handle,
                display_name: p.display_name,
                session_token: token,
            }),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    match state.passphrases.login(&body.handle, &body.passphrase).await {
        Ok((p, token)) => Json(SessionResponse {
            principal_id: p.id.to_string(),
            handle: p.handle,
            display_name: p.display_name,
            session_token: token,
        })
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn set_passphrase(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SetPassphraseBody>,
) -> impl IntoResponse {
    let token = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer)
    {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let ctx = match state.principal_resolver.resolve_bearer(token).await {
        Ok(c) => c,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match state
        .passphrases
        .set_passphrase(ctx.principal_id(), &body.passphrase)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}


// ----- Break Glass -----

async fn require_human(state: &AppState, headers: &HeaderMap) -> Result<tt_auth::AuthContext, StatusCode> {
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

async fn bg_get_settings(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let ctx = match require_human(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.break_glass.get_settings(ctx.principal_id()).await {
        Ok(s) => Json(s).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct BgSettingsBody {
    time_lock_enabled: Option<bool>,
    time_lock_delay_secs: Option<i64>,
    trustees_enabled: Option<bool>,
    trustee_threshold: Option<i32>,
    notify_email: Option<Option<String>>,
    notify_phone: Option<Option<String>>,
}

async fn bg_put_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BgSettingsBody>,
) -> impl IntoResponse {
    let ctx = match require_human(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state
        .break_glass
        .update_settings(
            ctx.principal_id(),
            body.time_lock_enabled,
            body.time_lock_delay_secs,
            body.trustees_enabled,
            body.trustee_threshold,
            body.notify_email,
            body.notify_phone,
        )
        .await
    {
        Ok(s) => Json(s).into_response(),
        Err(e) => map_err(e),
    }
}

async fn bg_list_trustees(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let ctx = match require_human(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.break_glass.list_trustees(ctx.principal_id()).await {
        Ok(t) => Json(t).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct AddTrusteeBody {
    label_commitment: String,
    trustee_pubkey: String,
    trustee_principal_id: Option<String>,
}

async fn bg_add_trustee(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AddTrusteeBody>,
) -> impl IntoResponse {
    let ctx = match require_human(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let linked = match body.trustee_principal_id {
        Some(s) => Some(match s.parse() {
            Ok(id) => id,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }),
        None => None,
    };
    match state
        .break_glass
        .add_trustee(ctx.principal_id(), &body.label_commitment, &body.trustee_pubkey, linked)
        .await
    {
        Ok(t) => (StatusCode::CREATED, Json(t)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn bg_remove_trustee(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let ctx = match require_human(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.break_glass.remove_trustee(ctx.principal_id(), id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct HandleBody {
    handle: String,
}

async fn bg_trigger_time_lock(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HandleBody>,
) -> impl IntoResponse {
    let fp = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    match state.break_glass.trigger_time_lock(&body.handle, Some(fp)).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct CancelBody {
    cancel_token: Option<String>,
}

async fn bg_cancel_time_lock(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Option<Json<CancelBody>>,
) -> impl IntoResponse {
    let session_pid = require_human(&state, &headers).await.ok().map(|c| c.principal_id());
    let token = body.and_then(|j| j.0.cancel_token);
    match state
        .break_glass
        .cancel_time_lock(id, token.as_deref(), session_pid)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

async fn bg_trigger_trustees(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HandleBody>,
) -> impl IntoResponse {
    let fp = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    match state
        .break_glass
        .trigger_trustee_recovery(&body.handle, Some(fp))
        .await
    {
        Ok(res) => Json(res).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct ApproveBody {
    trustee_id: Uuid,
    signature: String,
}

async fn bg_approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ApproveBody>,
) -> impl IntoResponse {
    let actor = require_human(&state, &headers).await.ok().map(|c| c.principal_id());
    match state
        .break_glass
        .approve_as_trustee(id, body.trustee_id, &body.signature, actor)
        .await
    {
        Ok(ev) => Json(ev).into_response(),
        Err(e) => map_err(e),
    }
}

async fn bg_get_event(State(state): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match state.break_glass.get_event(id).await {
        Ok(ev) => Json(ev).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Serialize)]
struct ClaimResponse {
    recovery_token: String,
}

async fn bg_claim(State(state): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match state.break_glass.claim_ready(id).await {
        Ok(recovery_token) => Json(ClaimResponse { recovery_token }).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct CompleteBody {
    recovery_token: String,
    new_passphrase: String,
}

async fn bg_complete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<CompleteBody>,
) -> impl IntoResponse {
    match state
        .break_glass
        .complete_recovery(id, &body.recovery_token, &body.new_passphrase)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

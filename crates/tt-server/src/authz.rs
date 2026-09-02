//! Shared HTTP authz helpers — keep route handlers thin.

use axum::http::{header, HeaderMap, StatusCode};
use tt_auth::AuthContext;
use tt_core::domain::OrgPermission;
use tt_core::ids::{OrgId, PrincipalId};

use crate::state::AppState;

pub async fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthContext, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.trim()))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .principal_resolver
        .resolve_bearer(token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

pub fn require_superadmin(state: &AppState, ctx: &AuthContext) -> Result<(), StatusCode> {
    let pid = ctx.principal_id().to_string();
    if state.config.is_superadmin(&pid) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

pub async fn require_org_perm(
    state: &AppState,
    org_id: OrgId,
    principal_id: PrincipalId,
    perm: OrgPermission,
) -> Result<(), StatusCode> {
    match state.orgs.require_permission(org_id, principal_id, perm).await {
        Ok(()) => Ok(()),
        Err(tt_messaging::MessagingError::Forbidden(_)) => Err(StatusCode::FORBIDDEN),
        Err(tt_messaging::MessagingError::NotFound) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn map_msg_err(e: tt_messaging::MessagingError) -> StatusCode {
    match e {
        tt_messaging::MessagingError::Forbidden(_) => StatusCode::FORBIDDEN,
        tt_messaging::MessagingError::NotFound => StatusCode::NOT_FOUND,
        tt_messaging::MessagingError::Conflict(_) => StatusCode::CONFLICT,
        tt_messaging::MessagingError::InvalidArgument(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

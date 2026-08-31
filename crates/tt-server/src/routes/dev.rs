//! Dev-only helpers (enabled when TT_DEV_BOOTSTRAP=1).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tt_core::domain::{Principal, PrincipalKind};
use tt_core::ids::PrincipalId;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/dev/bootstrap", post(bootstrap))
}

fn dev_enabled() -> bool {
    std::env::var("TT_DEV_BOOTSTRAP")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[derive(Debug, Deserialize)]
pub struct BootstrapBody {
    #[serde(default = "default_handle")]
    pub handle: String,
    #[serde(default = "default_name")]
    pub display_name: String,
    #[serde(default = "default_ws")]
    pub workspace_name: String,
    #[serde(default = "default_slug")]
    pub workspace_slug: String,
}

fn default_handle() -> String {
    "alice".into()
}
fn default_name() -> String {
    "Alice".into()
}
fn default_ws() -> String {
    "Turing Tarpit".into()
}
fn default_slug() -> String {
    "turing".into()
}

#[derive(Debug, Serialize)]
struct BootstrapResponse {
    principal_id: String,
    handle: String,
    session_token: String,
    workspace_id: String,
    workspace_slug: String,
    org_id: String,
    org_slug: String,
}

async fn bootstrap(
    State(state): State<AppState>,
    body: Option<Json<BootstrapBody>>,
) -> impl IntoResponse {
    if !dev_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "set TT_DEV_BOOTSTRAP=1 to enable" })),
        )
            .into_response();
    }

    let body = body.map(|j| j.0).unwrap_or(BootstrapBody {
        handle: default_handle(),
        display_name: default_name(),
        workspace_name: default_ws(),
        workspace_slug: default_slug(),
    });

    // Reuse existing principal if handle exists
    let principal = if let Ok(Some(p)) = state
        .auth_store
        .get_principal_by_handle(&body.handle)
        .await
    {
        p
    } else {
        let now = Utc::now();
        let p = Principal {
            id: PrincipalId::new(),
            kind: PrincipalKind::Human,
            display_name: body.display_name.clone(),
            handle: body.handle.clone(),
            avatar_url: None,
            email: None,
            owner_id: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = state.auth_store.create_principal(&p).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
        p
    };

    let session_token = match state
        .sessions
        .create_session(principal.id, Some("dev-bootstrap".into()), None)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Workspace: reuse by slug or create
    let workspace = match state.workspaces.list_for_principal(principal.id).await {
        Ok(list) if !list.is_empty() => list.into_iter().next().unwrap(),
        _ => match state
            .workspaces
            .create(
                body.workspace_name,
                body.workspace_slug.clone(),
                Some("Dev workspace".into()),
                principal.id,
            )
            .await
        {
            Ok(ws) => ws,
            Err(tt_messaging::MessagingError::AlreadyExists) => {
                // fetch by listing won't work if not member — try create with unique slug
                match state
                    .workspaces
                    .create(
                        format!("{} Home", body.display_name),
                        format!("{}-home", body.handle),
                        None,
                        principal.id,
                    )
                    .await
                {
                    Ok(ws) => ws,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": e.to_string() })),
                        )
                            .into_response();
                    }
                }
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        },
    };

    // Personal org + attach workspace
    let org = match state
        .orgs
        .ensure_personal(principal.id, &principal.handle)
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("org: {e}") })),
            )
                .into_response();
        }
    };
    let _ = state
        .orgs
        .attach_workspace(org.id, workspace.id, principal.id)
        .await;

    state
        .search
        .index_person(
            principal.id,
            &principal.handle,
            &principal.display_name,
            None,
        )
        .await;

    (
        StatusCode::CREATED,
        Json(BootstrapResponse {
            principal_id: principal.id.to_string(),
            handle: principal.handle,
            session_token,
            workspace_id: workspace.id.to_string(),
            workspace_slug: workspace.slug,
            org_id: org.id.to_string(),
            org_slug: org.slug,
        }),
    )
        .into_response()
}

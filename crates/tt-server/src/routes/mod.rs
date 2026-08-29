//! HTTP routes.

mod repos;
mod workspaces;
mod dev;
mod pull_requests;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tt_git::{git_http_router, GitHttpState};
use tt_messaging::{messaging_http_router, MessagingHttpState};

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let git_state = GitHttpState {
        repos: state.repos.clone(),
        resolver: state.principal_resolver.clone(),
    };
    let msg_state = MessagingHttpState {
        messaging: state.messaging.clone(),
        resolver: state.principal_resolver.clone(),
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(repos::router())
        .merge(workspaces::router())
        .merge(pull_requests::router())
        .merge(dev::router())
        .with_state(state)
        .merge(messaging_http_router(msg_state))
        .merge(git_http_router(git_state))
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "not_ready", "error": e.to_string() })),
        )
            .into_response(),
    }
}

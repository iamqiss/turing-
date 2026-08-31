//! HTTP routes.

mod repos;
mod workspaces;
mod dev;
mod pull_requests;
mod profiles;
mod issues;
mod reviews;
mod orgs;
mod billing;
mod compute;
mod search;
mod agents;
mod secrets;
mod media;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tt_git::{git_http_router, GitHttpState};
use tt_messaging::{messaging_http_router, MessagingHttpState};

use crate::push_observer::spawn_push_consumer;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let push_tx = spawn_push_consumer(state.profiles.events_store());
    let git_state = GitHttpState {
        repos: state.repos.clone(),
        resolver: state.principal_resolver.clone(),
        push_tx: Some(push_tx),
        deploy_pool: Some(state.db.clone()),
    };
    let msg_state = MessagingHttpState {
        messaging: state.messaging.clone(),
        resolver: state.principal_resolver.clone(),
        search: Some(state.search.clone()),
        media: Some(state.media.clone()),
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(repos::router())
        .merge(workspaces::router())
        .merge(pull_requests::router())
        .merge(profiles::router())
        .merge(issues::router())
        .merge(reviews::router())
        .merge(orgs::router())
        .merge(billing::router())
        .merge(compute::router())
        .merge(search::router())
        .merge(agents::router())
        .merge(media::router())
        .merge(secrets::router())
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

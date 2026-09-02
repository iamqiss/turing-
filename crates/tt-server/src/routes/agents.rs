//! Agent run API.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tt_agents::AgentError;
use tt_core::ids::{PrincipalId, WorkspaceId};
use uuid::Uuid;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/agent/runs", post(start_run).get(list_runs))
        .route("/api/agent/runs/:id", get(get_run))
        .route("/api/agent/runs/:id/tick", post(tick_run))
        .route("/api/agent/approvals/:id/resolve", post(resolve_approval))
}

#[derive(Debug, Deserialize)]
pub struct StartRunBody {
    pub goal: String,
    pub workspace_id: Option<String>,
    pub org_id: Option<String>,
    pub max_steps: Option<u32>,
    /// Agent principal; defaults to owner (human-as-agent for v1)
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    pub approve: bool,
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

fn map_err(e: AgentError) -> axum::response::Response {
    match e {
        AgentError::NotFound => StatusCode::NOT_FOUND.into_response(),
        AgentError::Forbidden(m) => (StatusCode::FORBIDDEN, m).into_response(),
        AgentError::Invalid(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        AgentError::ApprovalRequired => StatusCode::CONFLICT.into_response(),
        other => {
            tracing::error!(error = %other, "agent error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

fn run_json(r: &tt_core::domain::AgentRun) -> serde_json::Value {
    serde_json::json!({
        "id": r.id.to_string(),
        "status": r.status.as_str(),
        "goal": r.goal,
        "model": r.model,
        "step_count": r.step_count,
        "max_steps": r.max_steps,
        "result_summary": r.result_summary,
        "error": r.error,
        "workspace_id": r.workspace_id.map(|id| id.to_string()),
        "metadata": r.metadata,
        "created_at": r.created_at.to_rfc3339(),
        "finished_at": r.finished_at.map(|t| t.to_rfc3339()),
    })
}

async fn start_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<StartRunBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if let Some(ref oid) = body.org_id {
        if let Ok(org_id) = oid.parse::<tt_core::ids::OrgId>() {
            if state
                .orgs
                .require_permission(
                    org_id,
                    ctx.principal_id(),
                    tt_core::domain::OrgPermission::RunAgentSafe,
                )
                .await
                .is_err()
            {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }
    let ws = match body.workspace_id.as_deref() {
        Some(s) => match s.parse::<WorkspaceId>() {
            Ok(id) => Some(id),
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        },
        None => None,
    };
    if let Some(ws) = ws {
        if state
            .workspaces
            .ensure_member(ws, ctx.principal_id())
            .await
            .is_err()
        {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let agent_id = body
        .agent_id
        .as_deref()
        .and_then(|s| s.parse::<PrincipalId>().ok())
        .unwrap_or_else(|| ctx.principal_id());

    let org_id = body
        .org_id
        .as_deref()
        .and_then(|s| s.parse().ok());

    match state
        .agents
        .start_run(
            ctx.principal_id(),
            agent_id,
            body.goal,
            ws,
            org_id,
            body.max_steps,
        )
        .await
    {
        Ok(run) => {
            // Auto-tick once so callers get progress without a second hop
            let run = match state.agents.tick_run(run.id).await {
                Ok(r) => r,
                Err(e) => return map_err(e),
            };
            (StatusCode::CREATED, Json(run_json(&run))).into_response()
        }
        Err(e) => map_err(e),
    }
}

async fn list_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.agents.list_runs(ctx.principal_id(), 30).await {
        Ok(runs) => Json(runs.iter().map(run_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: Uuid = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.agents.get_run(id).await {
        Ok(run) if run.owner_id == ctx.principal_id() => Json(run_json(&run)).into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => map_err(e),
    }
}

async fn tick_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: Uuid = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.agents.get_run(id).await {
        Ok(run) if run.owner_id != ctx.principal_id() => {
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(e) => return map_err(e),
        _ => {}
    }
    match state.agents.tick_run(id).await {
        Ok(run) => Json(run_json(&run)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn resolve_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ResolveBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: Uuid = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state
        .agents
        .resolve_approval(id, ctx.principal_id(), body.approve)
        .await
    {
        Ok(run) => Json(run_json(&run)).into_response(),
        Err(e) => map_err(e),
    }
}

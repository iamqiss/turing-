//! Turing Tasks HTTP API — pipelines under `.tt/pipelines/`, billed as task minutes.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tt_core::TaskTrigger;
use tt_core::ids::{RepositoryId, WorkspaceId};
use uuid::Uuid;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/tasks/pipelines",
            get(list_pipelines).post(register_pipeline),
        )
        .route(
            "/api/workspaces/:workspace_id/repos/:repo_name/tasks/runs",
            get(list_runs).post(dispatch_run),
        )
        .route("/api/tasks/runs/:id", get(get_run))
        .route("/api/orgs/:slug/tasks/usage", get(org_usage))
}

async fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<tt_auth::AuthContext, StatusCode> {
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

async fn resolve_repo(
    state: &AppState,
    workspace_id: &str,
    repo_name: &str,
) -> Result<(tt_core::domain::Repository, Option<tt_core::ids::OrgId>), StatusCode> {
    let ws: WorkspaceId = workspace_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let name = repo_name.strip_suffix(".git").unwrap_or(repo_name);
    let repo = state
        .repos
        .get_by_name(ws, name)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let org_id: Option<tt_core::ids::OrgId> = sqlx::query_as::<_, (Option<Uuid>,)>(
        "SELECT org_id FROM workspaces WHERE id = $1",
    )
    .bind(ws.as_uuid())
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .and_then(|(o,)| o.map(tt_core::ids::OrgId::from_uuid));
    Ok((repo, org_id))
}

#[derive(Deserialize)]
struct RegisterBody {
    /// Pipeline name (also used as `.tt/pipelines/{name}.yml`)
    name: String,
    yaml: String,
    path: Option<String>,
}

async fn register_pipeline(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<RegisterBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (repo, org_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    let path = body
        .path
        .unwrap_or_else(|| format!(".tt/pipelines/{}.yml", body.name));
    match state
        .tasks
        .register_pipeline(repo.id, org_id, body.name, path, body.yaml)
        .await
    {
        Ok(p) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": p.id.to_string(),
                "name": p.name,
                "path": p.path,
                "enabled": p.enabled,
                "repository_id": p.repository_id.to_string(),
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn list_pipelines(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (repo, _) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    match state.tasks.list_pipelines(repo.id).await {
        Ok(list) => {
            let v: Vec<_> = list
                .into_iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id.to_string(),
                        "name": p.name,
                        "path": p.path,
                        "enabled": p.enabled,
                        "updated_at": p.updated_at.to_rfc3339(),
                    })
                })
                .collect();
            Json(v).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct DispatchBody {
    pipeline: Option<String>,
    /// manual | push | pull_request
    trigger: Option<String>,
    git_ref: Option<String>,
    git_sha: Option<String>,
    /// Optional ad-hoc YAML (registers if pipeline name given)
    yaml: Option<String>,
}

async fn dispatch_run(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DispatchBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (repo, org_id) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    if let Some(oid) = org_id {
        if let Err(e) = state.billing.assert_org_active(oid).await {
            return (StatusCode::FORBIDDEN, e.to_string()).into_response();
        }
    }
    let trigger = body
        .trigger
        .as_deref()
        .and_then(TaskTrigger::parse)
        .unwrap_or(TaskTrigger::Manual);
    match state
        .tasks
        .dispatch(
            repo.id,
            org_id,
            body.pipeline.as_deref(),
            trigger,
            body.git_ref,
            body.git_sha,
            Some(ctx.principal_id()),
            body.yaml,
        )
        .await
    {
        Ok(run) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": run.id.to_string(),
                "pipeline_name": run.pipeline_name,
                "status": run.status.as_str(),
                "trigger": run.trigger.as_str(),
                "billable_minutes": run.billable_minutes,
                "started_at": run.started_at.map(|t| t.to_rfc3339()),
                "finished_at": run.finished_at.map(|t| t.to_rfc3339()),
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

async fn list_runs(
    State(state): State<AppState>,
    Path((workspace_id, repo_name)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (repo, _) = match resolve_repo(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    let limit = q.limit.unwrap_or(30).clamp(1, 100);
    match state.tasks.list_runs(repo.id, limit).await {
        Ok(list) => {
            let v: Vec<_> = list
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id.to_string(),
                        "pipeline_name": r.pipeline_name,
                        "status": r.status.as_str(),
                        "trigger": r.trigger.as_str(),
                        "billable_minutes": r.billable_minutes,
                        "git_ref": r.git_ref,
                        "git_sha": r.git_sha,
                        "created_at": r.created_at.to_rfc3339(),
                        "finished_at": r.finished_at.map(|t| t.to_rfc3339()),
                    })
                })
                .collect();
            Json(v).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match state.tasks.get_run(id).await {
        Ok((run, jobs)) => Json(serde_json::json!({
            "id": run.id.to_string(),
            "pipeline_name": run.pipeline_name,
            "status": run.status.as_str(),
            "trigger": run.trigger.as_str(),
            "billable_minutes": run.billable_minutes,
            "git_ref": run.git_ref,
            "git_sha": run.git_sha,
            "error": run.error,
            "started_at": run.started_at.map(|t| t.to_rfc3339()),
            "finished_at": run.finished_at.map(|t| t.to_rfc3339()),
            "jobs": jobs.iter().map(|j| serde_json::json!({
                "id": j.id.to_string(),
                "name": j.name,
                "status": j.status.as_str(),
                "runs_on": j.runs_on,
                "billable_minutes": j.billable_minutes,
                "log_excerpt": j.log_excerpt,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn org_usage(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let period = tt_billing::BillingService::period_ym_now();
    let used = state
        .tasks
        .org_minutes_used(org.id, &period)
        .await
        .unwrap_or(0);
    let plan = state
        .billing
        .get_account(org.id)
        .await
        .map(|a| a.plan_id.as_str().to_string())
        .unwrap_or_else(|_| "free".into());
    let included = tt_pipelines::included_minutes(&plan);
    Json(serde_json::json!({
        "org": slug,
        "period_ym": period,
        "plan": plan,
        "minutes_used": used,
        "minutes_included": included,
        "minutes_overage": used.saturating_sub(included),
        "meter": "tasks.minutes",
        "overage_rate_micros_per_minute": 8_000,
    }))
    .into_response()
}

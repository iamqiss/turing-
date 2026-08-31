//! Compute HTTP API — nodes, workloads, start/stop.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post, patch};
use axum::{Json, Router};
use serde::Deserialize;
use tt_auth::extract_bearer;
use tt_compute::ComputeError;
use tt_core::domain::{RegionCode, ResourceSpec, WorkloadShape};
use tt_core::ids::{NodeId, WorkloadId};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/compute/nodes", post(register_node).get(list_nodes))
        .route("/api/compute/nodes/:id/heartbeat", post(heartbeat))
        .route(
            "/api/orgs/:slug/workloads",
            post(create_workload).get(list_workloads),
        )
        .route("/api/workloads/:id", get(get_workload))
        .route("/api/workloads/:id/start", post(start_workload))
        .route("/api/workloads/:id/stop", post(stop_workload))
        .route("/api/workloads/:id/instances", get(list_instances))
        .route("/api/instances/:id", get(get_instance))
        .route("/api/compute/networks/shared-nat", post(register_nat))
        .route("/api/orgs/:slug/deploy", post(deploy))
        .route(
            "/api/orgs/:slug/compute-quota",
            get(get_quota).patch(patch_quota),
        )
        .route(
            "/api/orgs/:slug/lifecycle-events",
            get(list_lifecycle),
        )
        .route("/api/compute/lifecycle-events", get(list_platform_lifecycle))
}

#[derive(Debug, Deserialize)]
pub struct RegisterNodeBody {
    pub name: String,
    pub region: String,
    pub physical_cores: u32,
    pub memory_mb: u32,
    #[serde(default)]
    pub reserved_cores: u32,
    #[serde(default)]
    pub reserved_memory_mb: u32,
    #[serde(default = "default_overcommit")]
    pub overcommit_ratio: f32,
}

fn default_overcommit() -> f32 {
    4.0
}

#[derive(Debug, Deserialize)]
pub struct RegionQuery {
    pub region: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkloadBody {
    pub name: String,
    pub shape: String,
    pub region: String,
    #[serde(default = "default_vcpu")]
    pub vcpu: u32,
    #[serde(default = "default_mem")]
    pub memory_mb: u32,
    #[serde(default = "default_disk")]
    pub disk_gb: u32,
    pub image_template_id: Option<String>,
    pub repository_id: Option<String>,
    pub git_ref: Option<String>,
}

fn default_vcpu() -> u32 {
    1
}
fn default_mem() -> u32 {
    512
}
fn default_disk() -> u32 {
    10
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatBody {
    pub endpoint: String,
    /// Node agent token from registration (tt_node_...)
    pub agent_token: Option<String>,
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
        .map_err(|e| match e {
            tt_messaging::MessagingError::Forbidden(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        })?;
    Ok((ctx, org))
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

fn map_err(e: ComputeError) -> axum::response::Response {
    match e {
        ComputeError::NotFound => StatusCode::NOT_FOUND.into_response(),
        ComputeError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        ComputeError::Forbidden(m) => (StatusCode::FORBIDDEN, m).into_response(),
        ComputeError::QuotaExceeded(m) | ComputeError::NoCapacity(m) | ComputeError::Conflict(m) => {
            (StatusCode::CONFLICT, m).into_response()
        }
        ComputeError::Provider(m) => (StatusCode::BAD_GATEWAY, m).into_response(),
        other => {
            tracing::error!(error = %other, "compute error");
            (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
        }
    }
}

async fn register_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RegisterNodeBody>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(region) = RegionCode::parse(&body.region) else {
        return (StatusCode::BAD_REQUEST, "invalid region").into_response();
    };
    match state
        .compute
        .register_node(
            body.name,
            region,
            body.physical_cores,
            body.memory_mb,
            body.reserved_cores,
            body.reserved_memory_mb,
            body.overcommit_ratio,
        )
        .await
    {
        Ok((n, agent_token)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": n.id.to_string(),
                "name": n.name,
                "region": n.region.as_str(),
                "physical_cores": n.physical_cores,
                "sellable_cores": n.sellable_cores(),
                "capacity_1vcpu": n.capacity_1vcpu(),
                "overcommit_ratio": n.overcommit_ratio,
                "status": n.status.as_str(),
                // Shown once — store securely on the metal host (TT_NODE_AGENT_TOKEN)
                "agent_token": agent_token,
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_nodes(
    State(state): State<AppState>,
    Query(q): Query<RegionQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let region = q.region.as_deref().and_then(RegionCode::parse);
    match state.compute.list_nodes(region).await {
        Ok(list) => Json(
            list.iter()
                .map(|n| {
                    serde_json::json!({
                        "id": n.id.to_string(),
                        "name": n.name,
                        "region": n.region.as_str(),
                        "status": n.status.as_str(),
                        "physical_cores": n.physical_cores,
                        "capacity_1vcpu": n.capacity_1vcpu(),
                        "last_heartbeat": n.last_heartbeat.map(|t| t.to_rfc3339()),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<HeartbeatBody>,
) -> impl IntoResponse {
    // Node agents may use a shared secret later; open for lab
    let nid: NodeId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.compute.heartbeat(nid, &body.endpoint, body.agent_token.as_deref()).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => map_err(e),
    }
}

async fn create_workload(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateWorkloadBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                format!("org: {e}"),
            )
                .into_response();
        }
    };
    let Some(shape) = WorkloadShape::parse(&body.shape) else {
        return (StatusCode::BAD_REQUEST, "shape: serverless|vm|kata").into_response();
    };
    let Some(region) = RegionCode::parse(&body.region) else {
        return (StatusCode::BAD_REQUEST, "invalid region").into_response();
    };
    let image = body
        .image_template_id
        .as_deref()
        .and_then(|s| s.parse().ok());
    let repo = body.repository_id.as_deref().and_then(|s| s.parse().ok());
    match state
        .compute
        .create_workload(
            org.id,
            ctx.principal_id(),
            body.name,
            shape,
            ResourceSpec {
                vcpu: body.vcpu,
                memory_mb: body.memory_mb,
                disk_gb: body.disk_gb,
            },
            region,
            image,
            repo,
            body.git_ref,
        )
        .await
    {
        Ok(w) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": w.id.to_string(),
                "name": w.name,
                "shape": w.shape.as_str(),
                "status": w.status.as_str(),
                "region": w.region.as_str(),
                "resources": {
                    "vcpu": w.resources.vcpu,
                    "memory_mb": w.resources.memory_mb,
                    "disk_gb": w.resources.disk_gb,
                },
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_workloads(
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
    match state.compute.list_workloads(org.id).await {
        Ok(list) => Json(
            list.iter()
                .map(|w| {
                    serde_json::json!({
                        "id": w.id.to_string(),
                        "name": w.name,
                        "shape": w.shape.as_str(),
                        "status": w.status.as_str(),
                        "region": w.region.as_str(),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_workload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let wid: WorkloadId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.compute.get_workload(wid).await {
        Ok(w) => {
            if state
                .orgs
                .require_active_member(w.org_id, ctx.principal_id())
                .await
                .is_err()
            {
                return StatusCode::NOT_FOUND.into_response();
            }
            Json(serde_json::json!({
                "id": w.id.to_string(),
                "org_id": w.org_id.to_string(),
                "name": w.name,
                "shape": w.shape.as_str(),
                "status": w.status.as_str(),
                "region": w.region.as_str(),
                "resources": w.resources,
                "git_ref": w.git_ref,
            }))
            .into_response()
        }
        Err(e) => map_err(e),
    }
}

async fn start_workload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let wid: WorkloadId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let wl = match state.compute.get_workload(wid).await {
        Ok(w) => w,
        Err(e) => return map_err(e),
    };
    // Tenant isolation: foreign org → uniform 404
    if state
        .orgs
        .require_active_member(wl.org_id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match state.compute.start(wid).await {
        Ok(inst) => Json(serde_json::json!({
            "instance_id": inst.id.to_string(),
            "node_id": inst.node_id.to_string(),
            "driver": inst.driver.as_str(),
            "status": inst.status.as_str(),
            "nat_public_ip": inst.nat_public_ip,
            "nat_port_start": inst.nat_port_start,
            "nat_port_end": inst.nat_port_end,
            "ipv6": inst.ipv6,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn stop_workload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let wid: WorkloadId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let wl = match state.compute.get_workload(wid).await {
        Ok(w) => w,
        Err(e) => return map_err(e),
    };
    if state
        .orgs
        .require_active_member(wl.org_id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match state.compute.stop(wid).await {
        Ok(()) => Json(serde_json::json!({ "stopped": true })).into_response(),
        Err(e) => map_err(e),
    }
}


#[derive(Debug, Deserialize)]
pub struct RegisterNatBody {
    pub region: String,
    pub ip: String,
    pub node_id: Option<String>,
}

async fn register_nat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RegisterNatBody>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(region) = RegionCode::parse(&body.region) else {
        return (StatusCode::BAD_REQUEST, "invalid region").into_response();
    };
    let node_id = match body.node_id.as_deref() {
        Some(s) => match s.parse::<NodeId>() {
            Ok(id) => Some(id),
            Err(_) => return (StatusCode::BAD_REQUEST, "invalid node_id").into_response(),
        },
        None => None,
    };
    match state
        .compute
        .register_shared_nat(region, &body.ip, node_id)
        .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "subnet_id": id.to_string(),
                "region": body.region,
                "ip": body.ip,
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_instances(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let wid: WorkloadId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.compute.list_instances(wid).await {
        Ok(list) => Json(
            list.iter()
                .map(|i| {
                    serde_json::json!({
                        "id": i.id.to_string(),
                        "node_id": i.node_id.to_string(),
                        "driver": i.driver.as_str(),
                        "status": i.status.as_str(),
                        "nat_public_ip": i.nat_public_ip,
                        "nat_port_start": i.nat_port_start,
                        "nat_port_end": i.nat_port_end,
                        "ipv6": i.ipv6,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let iid: tt_core::ids::InstanceId = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.compute.get_instance(iid).await {
        Ok(i) => Json(serde_json::json!({
            "id": i.id.to_string(),
            "workload_id": i.workload_id.to_string(),
            "node_id": i.node_id.to_string(),
            "driver": i.driver.as_str(),
            "status": i.status.as_str(),
            "nat_public_ip": i.nat_public_ip,
            "nat_port_start": i.nat_port_start,
            "nat_port_end": i.nat_port_end,
            "ipv6": i.ipv6,
            "dedicated_ipv4": i.dedicated_ipv4,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}


#[derive(Debug, Deserialize)]
pub struct DeployBody {
    pub name: String,
    pub shape: String,
    pub region: String,
    pub repository_id: String,
    pub git_ref: String,
    pub git_sha: Option<String>,
    #[serde(default = "default_vcpu")]
    pub vcpu: u32,
    #[serde(default = "default_mem")]
    pub memory_mb: u32,
    #[serde(default = "default_disk")]
    pub disk_gb: u32,
}

async fn deploy(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DeployBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let Some(shape) = WorkloadShape::parse(&body.shape) else {
        return (StatusCode::BAD_REQUEST, "shape: serverless|vm|kata").into_response();
    };
    let Some(region) = RegionCode::parse(&body.region) else {
        return (StatusCode::BAD_REQUEST, "invalid region").into_response();
    };
    let repo_id = match body.repository_id.parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid repository_id").into_response(),
    };
    match state
        .compute
        .deploy_from_git(
            org.id,
            ctx.principal_id(),
            body.name,
            shape,
            region,
            ResourceSpec {
                vcpu: body.vcpu,
                memory_mb: body.memory_mb,
                disk_gb: body.disk_gb,
            },
            repo_id,
            body.git_ref,
            body.git_sha,
            None,
        )
        .await
    {
        Ok((w, inst)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "workload_id": w.id.to_string(),
                "instance_id": inst.id.to_string(),
                "status": inst.status.as_str(),
                "driver": inst.driver.as_str(),
                "nat_public_ip": inst.nat_public_ip,
                "nat_port_start": inst.nat_port_start,
                "nat_port_end": inst.nat_port_end,
                "git_ref": w.git_ref,
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}


#[derive(Debug, Deserialize)]
pub struct PatchQuotaBody {
    pub max_workloads: Option<u32>,
    pub max_vcpu_total: Option<u32>,
    pub max_concurrent_running: Option<u32>,
}

async fn get_quota(
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
    match state.compute.get_quota_usage(org.id).await {
        Ok(q) => Json(serde_json::json!({
            "org_id": org.id.to_string(),
            "slug": slug,
            "max_workloads": q.max_workloads,
            "max_vcpu_total": q.max_vcpu_total,
            "max_concurrent_running": q.max_concurrent_running,
            "workloads_used": q.workloads_used,
            "vcpu_used": q.vcpu_used,
            "running_used": q.running_used,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

async fn patch_quota(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PatchQuotaBody>,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    // Merge with current limits
    let cur = match state.compute.get_quota_usage(org.id).await {
        Ok(q) => q,
        Err(e) => return map_err(e),
    };
    let max_workloads = body.max_workloads.unwrap_or(cur.max_workloads);
    let max_vcpu_total = body.max_vcpu_total.unwrap_or(cur.max_vcpu_total);
    let max_concurrent_running = body
        .max_concurrent_running
        .unwrap_or(cur.max_concurrent_running);
    match state
        .compute
        .set_quota(org.id, max_workloads, max_vcpu_total, max_concurrent_running)
        .await
    {
        Ok(q) => Json(serde_json::json!({
            "org_id": org.id.to_string(),
            "slug": slug,
            "max_workloads": q.max_workloads,
            "max_vcpu_total": q.max_vcpu_total,
            "max_concurrent_running": q.max_concurrent_running,
            "workloads_used": q.workloads_used,
            "vcpu_used": q.vcpu_used,
            "running_used": q.running_used,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}


#[derive(Debug, Deserialize)]
pub struct LifecycleQuery {
    pub days: Option<u32>,
    pub limit: Option<u32>,
}

async fn list_lifecycle(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<LifecycleQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let days = q.days.unwrap_or(30).min(365);
    let limit = q.limit.unwrap_or(100).min(500);
    let since = chrono::Utc::now() - chrono::Duration::days(days as i64);
    match state.lifecycle.list_for_org(org.id, since, limit).await {
        Ok(events) => Json(
            events
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "ts": e.ts.to_rfc3339(),
                        "kind": e.kind.as_str(),
                        "org_id": e.org_id.map(|id| id.to_string()),
                        "principal_id": e.principal_id.map(|id| id.to_string()),
                        "workload_id": e.workload_id.map(|id| id.to_string()),
                        "instance_id": e.instance_id.map(|id| id.to_string()),
                        "node_id": e.node_id.map(|id| id.to_string()),
                        "region": e.region,
                        "metadata": e.metadata,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list lifecycle");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_platform_lifecycle(
    State(state): State<AppState>,
    Query(q): Query<LifecycleQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if require_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let days = q.days.unwrap_or(30).min(365);
    let limit = q.limit.unwrap_or(100).min(500);
    let since = chrono::Utc::now() - chrono::Duration::days(days as i64);
    match state.lifecycle.list_platform(since, limit).await {
        Ok(events) => Json(
            events
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "ts": e.ts.to_rfc3339(),
                        "kind": e.kind.as_str(),
                        "org_id": e.org_id.map(|id| id.to_string()),
                        "node_id": e.node_id.map(|id| id.to_string()),
                        "region": e.region,
                        "metadata": e.metadata,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list platform lifecycle");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

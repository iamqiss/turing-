//! Git Smart HTTP protocol.
//!
//! Implements the minimal surface for `git clone` / `git fetch` / `git push`:
//! - `GET  .../info/refs?service=git-upload-pack|git-receive-pack`
//! - `POST .../git-upload-pack`
//! - `POST .../git-receive-pack`
//!
//! Uses the system `git` binary for pack negotiation (reliable, matches client expectations).
//! Metadata and access control stay in Turing Tarpit.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{debug, warn};
use tt_auth::{extract_bearer, PrincipalResolver};
use tt_core::ids::WorkspaceId;

use crate::analytics::{analyze_recent_commits, PushAnalysis};
use crate::error::GitError;
use crate::repo::RepoService;
use tt_core::ids::{PrincipalId, RepositoryId};
use uuid::Uuid;


#[derive(Clone)]
pub struct GitHttpState {
    pub repos: Arc<RepoService>,
    pub resolver: Arc<PrincipalResolver>,
    pub push_tx: Option<tokio::sync::mpsc::UnboundedSender<PushAnalysis>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct InfoRefsQuery {
    pub service: String,
}

pub fn router(state: GitHttpState) -> Router {
    Router::new()
        .route(
            "/git/:workspace_id/:repo_name/info/refs",
            get(info_refs),
        )
        .route(
            "/git/:workspace_id/:repo_name/git-upload-pack",
            post(upload_pack),
        )
        .route(
            "/git/:workspace_id/:repo_name/git-receive-pack",
            post(receive_pack),
        )
        .with_state(state)
}

/// Clone URL path shape: `/git/{workspace_id}/{repo_name}.git` or without `.git`
fn normalize_name(name: &str) -> &str {
    name.strip_suffix(".git").unwrap_or(name)
}

async fn resolve_repo_path(
    state: &GitHttpState,
    workspace_id: &str,
    repo_name: &str,
) -> Result<(PathBuf, bool), Response> {
    let ws: WorkspaceId = workspace_id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid workspace_id").into_response())?;
    let name = normalize_name(repo_name);
    let repo = state
        .repos
        .get_by_name(ws, name)
        .await
        .map_err(|e| match e {
            GitError::NotFound => StatusCode::NOT_FOUND.into_response(),
            other => {
                warn!(error = %other, "repo lookup failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        })?;
    let path = state.repos.disk_path(repo.workspace_id, &repo.name);
    if !path.exists() {
        return Err(StatusCode::NOT_FOUND.into_response());
    }
    Ok((path, repo.is_private))
}


async fn authorize_principal(
    state: &GitHttpState,
    headers: &HeaderMap,
    is_private: bool,
    write: bool,
) -> Result<PrincipalId, Response> {
    if !is_private && !write {
        return Ok(PrincipalId::from_uuid(Uuid::nil()));
    }
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer);
    let Some(token) = auth else {
        return Err((
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "authentication required",
        )
            .into_response());
    };
    let ctx = state.resolver.resolve_bearer(token).await.map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            format!("unauthorized: {e}"),
        )
            .into_response()
    })?;
    Ok(ctx.principal_id())
}

async fn authorize(
    state: &GitHttpState,
    headers: &HeaderMap,
    is_private: bool,
    write: bool,
) -> Result<(), Response> {
    if !is_private && !write {
        // Public read
        return Ok(());
    }
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer);
    let Some(token) = auth else {
        return Err((
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "authentication required",
        )
            .into_response());
    };
    state
        .resolver
        .resolve_bearer(token)
        .await
        .map_err(|e| {
            (
                StatusCode::UNAUTHORIZED,
                format!("unauthorized: {e}"),
            )
                .into_response()
        })?;
    // Workspace-level ACL can be tightened later
    Ok(())
}

async fn info_refs(
    State(state): State<GitHttpState>,
    AxumPath((workspace_id, repo_name)): AxumPath<(String, String)>,
    Query(q): Query<InfoRefsQuery>,
    headers: HeaderMap,
) -> Response {
    let service = q.service.as_str();
    if service != "git-upload-pack" && service != "git-receive-pack" {
        return (StatusCode::FORBIDDEN, "unsupported service").into_response();
    }
    let write = service == "git-receive-pack";

    let (path, is_private) = match resolve_repo_path(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Err(r) = authorize(&state, &headers, is_private, write).await {
        return r;
    }

    match run_advertise(&path, service).await {
        Ok(body) => {
            let content_type = format!("application/x-{service}-advertisement");
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, content_type),
                    (header::CACHE_CONTROL, "no-cache".into()),
                ],
                body,
            )
                .into_response()
        }
        Err(e) => {
            warn!(error = %e, "info/refs failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn upload_pack(
    State(state): State<GitHttpState>,
    AxumPath((workspace_id, repo_name)): AxumPath<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (path, is_private) = match resolve_repo_path(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Err(r) = authorize(&state, &headers, is_private, false).await {
        return r;
    }
    proxy_pack(&path, "git-upload-pack", body).await
}

async fn receive_pack(
    State(state): State<GitHttpState>,
    AxumPath((workspace_id, repo_name)): AxumPath<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (path, is_private) = match resolve_repo_path(&state, &workspace_id, &repo_name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let principal = match authorize_principal(&state, &headers, is_private, true).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let response = proxy_pack(&path, "git-receive-pack", body).await;

    // Post-push analytics (best-effort; never fail the push)
    if response.status().is_success() {
        if let Some(tx) = state.push_tx.clone() {
            let repos = state.repos.clone();
            let path = path.clone();
            let ws = workspace_id.clone();
            let name = normalize_name(&repo_name).to_string();
            tokio::spawn(async move {
                let Ok(ws_id) = ws.parse::<WorkspaceId>() else { return };
                let Ok(repo) = repos.get_by_name(ws_id, &name).await else { return };
                match analyze_recent_commits(&path, ws_id, repo.id, principal, 15).await {
                    Ok(analysis) => { let _ = tx.send(analysis); },
                    Err(e) => warn!(error = %e, "push analytics failed"),
                }
            });
        }
    }
    response
}

/// pkt-line advertise refs for smart HTTP.
async fn run_advertise(repo: &Path, service: &str) -> anyhow::Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg(service.strip_prefix("git-").unwrap_or(service))
        .arg("--stateless-rpc")
        .arg("--advertise-refs")
        .arg(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .await?;
    let status = child.wait().await?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut stderr).await;
        }
        anyhow::bail!("git {service} advertise failed: {stderr}");
    }

    // Smart HTTP requires a service header pkt-line before the refs
    let mut out = Vec::new();
    let header = format!("# service={service}\n");
    out.extend_from_slice(pkt_line(&header).as_slice());
    out.extend_from_slice(b"0000"); // flush
    out.extend_from_slice(&stdout);
    Ok(out)
}

async fn proxy_pack(repo: &Path, service: &str, body: Body) -> Response {
    let bytes = match axum::body::to_bytes(body, 100 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let cmd = service.strip_prefix("git-").unwrap_or(service);
    let mut child = match Command::new("git")
        .arg(cmd)
        .arg("--stateless-rpc")
        .arg(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "failed to spawn git");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(&bytes).await {
            warn!(error = %e, "write to git stdin failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let mut stdout = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut stdout).await;
    }
    let status = match child.wait().await {
        Ok(s) => s,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if !status.success() {
        debug!(?status, service, "git pack command exited non-zero");
    }

    let content_type = format!("application/x-{service}-result");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        stdout,
    )
        .into_response()
}

fn pkt_line(s: &str) -> Vec<u8> {
    let len = s.len() + 4;
    let mut v = format!("{len:04x}").into_bytes();
    v.extend_from_slice(s.as_bytes());
    v
}

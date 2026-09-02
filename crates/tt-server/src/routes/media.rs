//! Media upload/download — avatars + channel attachments.

use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/media/:id", get(get_media))
        .route("/api/me/avatar", post(upload_avatar))
        .route(
            "/api/workspaces/:workspace_id/media",
            post(upload_attachment),
        )
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

fn media_json(m: &tt_storage::MediaObject, public_base: &str) -> serde_json::Value {
    serde_json::json!({
        "id": m.id.to_string(),
        "kind": m.kind,
        "filename": m.filename,
        "content_type": m.content_type,
        "size_bytes": m.size_bytes,
        "url": format!("{}/{}", public_base.trim_end_matches('/'), m.id),
        "created_at": m.created_at.to_rfc3339(),
    })
}

async fn get_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let id: Uuid = match id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.media.get_bytes(id).await {
        Ok((meta, bytes)) => {
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, meta.content_type),
                    (
                        header::CACHE_CONTROL,
                        "public, max-age=86400".into(),
                    ),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("inline; filename=\"{}\"", meta.filename.replace('"', "")),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(tt_storage::StorageError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "media get");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn read_multipart_file(
    multipart: &mut Multipart,
) -> Result<(String, String, Vec<u8>), StatusCode> {
    let mut filename = "upload.bin".to_string();
    let mut content_type = "application/octet-stream".to_string();
    let mut data = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "avatar" || name.is_empty() || data.is_empty() {
            if let Some(fnm) = field.file_name() {
                filename = fnm.to_string();
            }
            if let Some(ct) = field.content_type() {
                content_type = ct.to_string();
            }
            data = field
                .bytes()
                .await
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .to_vec();
        }
    }
    if data.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((filename, content_type, data))
}

async fn upload_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let (filename, content_type, data) = match read_multipart_file(&mut multipart).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    match state
        .media
        .upload_avatar(ctx.principal_id(), &filename, &content_type, &data)
        .await
    {
        Ok(m) => (
            StatusCode::CREATED,
            Json(media_json(&m, "/api/media")),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "avatar upload");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}

async fn upload_attachment(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let ws: Uuid = match workspace_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    // Membership check
    let ws_id = match workspace_id.parse::<tt_core::ids::WorkspaceId>() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if state
        .workspaces
        .ensure_member(ws_id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (filename, content_type, data) = match read_multipart_file(&mut multipart).await {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    match state
        .media
        .upload_attachment(ctx.principal_id(), ws, &filename, &content_type, &data)
        .await
    {
        Ok(m) => {
            // Best-effort usage: workspace → org
            if let Ok(Some((org_id,))) = sqlx::query_as::<_, (uuid::Uuid,)>(
                "SELECT org_id FROM workspaces WHERE id = $1 AND org_id IS NOT NULL",
            )
            .bind(ws)
            .fetch_optional(&state.db)
            .await
            {
                let _ = state
                    .billing
                    .emit_media_bytes(
                        tt_core::ids::OrgId::from_uuid(org_id),
                        m.size_bytes,
                        Some(m.id.to_string()),
                    )
                    .await;
            }
            (
                StatusCode::CREATED,
                Json(media_json(&m, "/api/media")),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "attachment upload");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}

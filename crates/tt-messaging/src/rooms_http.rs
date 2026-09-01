//! HTTP API for Rooms.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tt_auth::PrincipalResolver;
use tt_core::domain::{Room, RoomPost, RoomPostKind, RoomVisibility};
use uuid::Uuid;

use crate::error::MessagingError;
use crate::rooms::RoomService;

#[derive(Clone)]
pub struct RoomsHttpState {
    pub rooms: Arc<RoomService>,
    pub resolver: Arc<PrincipalResolver>,
}

pub fn router(state: RoomsHttpState) -> Router {
    Router::new()
        .route("/api/rooms", get(discover).post(create))
        .route("/api/rooms/mine", get(mine))
        .route("/api/rooms/:slug", get(get_room))
        .route("/api/rooms/:slug/join", post(join))
        .route("/api/rooms/:slug/leave", post(leave))
        .route("/api/rooms/:slug/posts", get(feed).post(post_to_room))
        .with_state(state)
}

async fn auth(
    state: &RoomsHttpState,
    headers: &HeaderMap,
) -> std::result::Result<tt_auth::AuthContext, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.trim()))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .resolver
        .resolve_bearer(token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

fn map_err(e: MessagingError) -> axum::response::Response {
    match e {
        MessagingError::NotFound => StatusCode::NOT_FOUND.into_response(),
        MessagingError::Forbidden(m) | MessagingError::PermissionDenied(m) => {
            (StatusCode::FORBIDDEN, m).into_response()
        }
        MessagingError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        MessagingError::Conflict(m) => (StatusCode::CONFLICT, m).into_response(),
        MessagingError::AlreadyExists => (StatusCode::CONFLICT, "already exists").into_response(),
        other => {
            tracing::error!(error = %other, "rooms");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Serialize)]
struct RoomJson {
    id: String,
    slug: String,
    title: String,
    description: Option<String>,
    visibility: String,
    member_count: u32,
    created_by: String,
    created_at: String,
}

fn room_json(r: &Room) -> RoomJson {
    RoomJson {
        id: r.id.to_string(),
        slug: r.slug.clone(),
        title: r.title.clone(),
        description: r.description.clone(),
        visibility: match r.visibility {
            RoomVisibility::Public => "public".into(),
            RoomVisibility::Private => "private".into(),
        },
        member_count: r.member_count,
        created_by: r.created_by.to_string(),
        created_at: r.created_at.to_rfc3339(),
    }
}

#[derive(Serialize)]
struct PostJson {
    id: String,
    room_id: String,
    author_id: String,
    kind: String,
    content: String,
    media_id: Option<String>,
    created_at: String,
}

fn post_json(p: &RoomPost) -> PostJson {
    PostJson {
        id: p.id.to_string(),
        room_id: p.room_id.to_string(),
        author_id: p.author_id.to_string(),
        kind: match p.kind {
            RoomPostKind::Text => "text".into(),
            RoomPostKind::VoiceNote => "voice_note".into(),
            RoomPostKind::VideoClip => "video_clip".into(),
            RoomPostKind::Attachment => "attachment".into(),
        },
        content: if p.is_deleted { String::new() } else { p.content.clone() },
        media_id: p.media_id.map(|m| m.to_string()),
        created_at: p.created_at.to_rfc3339(),
    }
}

#[derive(Deserialize)]
struct DiscoverQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn discover(
    State(state): State<RoomsHttpState>,
    headers: HeaderMap,
    Query(q): Query<DiscoverQuery>,
) -> impl IntoResponse {
    let _ = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state
        .rooms
        .discover(q.limit.unwrap_or(30), q.offset.unwrap_or(0))
        .await
    {
        Ok(list) => Json(list.iter().map(room_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Deserialize)]
struct CreateBody {
    slug: String,
    title: String,
    description: Option<String>,
    #[serde(default = "default_public")]
    visibility: String,
}

fn default_public() -> String {
    "public".into()
}

async fn create(
    State(state): State<RoomsHttpState>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let vis = match body.visibility.as_str() {
        "private" => RoomVisibility::Private,
        _ => RoomVisibility::Public,
    };
    match state
        .rooms
        .create(
            ctx.principal_id(),
            &body.slug,
            &body.title,
            body.description,
            vis,
        )
        .await
    {
        Ok(r) => (StatusCode::CREATED, Json(room_json(&r))).into_response(),
        Err(e) => map_err(e),
    }
}

async fn mine(
    State(state): State<RoomsHttpState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.rooms.mine(ctx.principal_id()).await {
        Ok(list) => Json(list.iter().map(room_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_room(
    State(state): State<RoomsHttpState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let _ = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.rooms.get(&slug).await {
        Ok(r) => Json(room_json(&r)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn join(
    State(state): State<RoomsHttpState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.rooms.join(&slug, ctx.principal_id()).await {
        Ok(r) => Json(room_json(&r)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn leave(
    State(state): State<RoomsHttpState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    match state.rooms.leave(&slug, ctx.principal_id()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Deserialize)]
struct FeedQuery {
    before: Option<String>,
    limit: Option<i64>,
}

async fn feed(
    State(state): State<RoomsHttpState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(q): Query<FeedQuery>,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let before = q.before.and_then(|s| s.parse::<Uuid>().ok());
    match state
        .rooms
        .feed(&slug, ctx.principal_id(), before, q.limit.unwrap_or(40))
        .await
    {
        Ok(list) => Json(list.iter().map(post_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Deserialize)]
struct PostBody {
    #[serde(default = "default_text")]
    kind: String,
    #[serde(default)]
    content: String,
    media_id: Option<String>,
}

fn default_text() -> String {
    "text".into()
}

async fn post_to_room(
    State(state): State<RoomsHttpState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PostBody>,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let kind = match body.kind.as_str() {
        "voice_note" => RoomPostKind::VoiceNote,
        "video_clip" => RoomPostKind::VideoClip,
        "attachment" => RoomPostKind::Attachment,
        _ => RoomPostKind::Text,
    };
    let media_id = body.media_id.and_then(|s| s.parse().ok());
    match state
        .rooms
        .post(&slug, ctx.principal_id(), kind, body.content, media_id)
        .await
    {
        Ok(p) => (StatusCode::CREATED, Json(post_json(&p))).into_response(),
        Err(e) => map_err(e),
    }
}

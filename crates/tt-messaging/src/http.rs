//! HTTP API for messaging.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tt_auth::{extract_bearer, PrincipalResolver};
use tt_core::domain::ChannelKind;
use tt_core::ids::{ChannelId, MessageId, PrincipalId, WorkspaceId};

use crate::error::MessagingError;
use crate::service::MessagingService;

#[derive(Clone)]
pub struct MessagingHttpState {
    pub messaging: Arc<MessagingService>,
    pub resolver: Arc<PrincipalResolver>,
    /// Optional Meilisearch indexer (set by tt-server)
    pub search: Option<Arc<tt_search::SearchService>>,
}

pub fn router(state: MessagingHttpState) -> Router {
    Router::new()
        .route(
            "/api/workspaces/:workspace_id/channels",
            post(create_channel).get(list_channels),
        )
        .route(
            "/api/channels/:channel_id",
            get(get_channel),
        )
        .route(
            "/api/channels/:channel_id/join",
            post(join_channel),
        )
        .route(
            "/api/channels/:channel_id/leave",
            post(leave_channel),
        )
        .route(
            "/api/channels/:channel_id/messages",
            post(send_message).get(list_messages),
        )
        .route(
            "/api/messages/:message_id",
            axum::routing::patch(edit_message).delete(delete_message),
        )
        .route(
            "/api/messages/:message_id/reactions",
            post(add_reaction).delete(remove_reaction),
        )
        .with_state(state)
}

async fn auth(
    state: &MessagingHttpState,
    headers: &HeaderMap,
) -> Result<tt_auth::AuthContext, StatusCode> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer)
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
        MessagingError::PermissionDenied(m) | MessagingError::Forbidden(m) => (StatusCode::FORBIDDEN, m).into_response(),
        MessagingError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        MessagingError::Conflict(m) => (StatusCode::CONFLICT, m).into_response(),
        MessagingError::AlreadyExists => StatusCode::CONFLICT.into_response(),
        MessagingError::Other(err) => {
            tracing::error!(error = %err, "messaging error");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct CreateChannelBody {
    kind: String,
    name: Option<String>,
    topic: Option<String>,
    member_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ChannelJson {
    id: String,
    workspace_id: String,
    kind: String,
    name: Option<String>,
    topic: Option<String>,
    is_archived: bool,
    created_by: String,
}

fn channel_json(c: &tt_core::domain::Channel) -> ChannelJson {
    ChannelJson {
        id: c.id.to_string(),
        workspace_id: c.workspace_id.to_string(),
        kind: format!("{:?}", c.kind).to_lowercase(),
        name: c.name.clone(),
        topic: c.topic.clone(),
        is_archived: c.is_archived,
        created_by: c.created_by.to_string(),
    }
}

#[derive(Debug, Serialize)]
struct MessageJson {
    id: String,
    channel_id: String,
    thread_id: Option<String>,
    author_id: String,
    content: String,
    reply_to_id: Option<String>,
    is_edited: bool,
    is_deleted: bool,
    created_at: String,
}

fn message_json(m: &tt_core::domain::Message) -> MessageJson {
    MessageJson {
        id: m.id.to_string(),
        channel_id: m.channel_id.to_string(),
        thread_id: m.thread_id.map(|t| t.to_string()),
        author_id: m.author_id.to_string(),
        content: if m.is_deleted {
            String::new()
        } else {
            m.content.clone()
        },
        reply_to_id: m.reply_to_id.map(|r| r.to_string()),
        is_edited: m.is_edited,
        is_deleted: m.is_deleted,
        created_at: m.created_at.to_rfc3339(),
    }
}

async fn create_channel(
    State(state): State<MessagingHttpState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateChannelBody>,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let kind = match body.kind.as_str() {
        "public" => ChannelKind::Public,
        "private" => ChannelKind::Private,
        "direct" => ChannelKind::Direct,
        _ => return (StatusCode::BAD_REQUEST, "kind must be public|private|direct").into_response(),
    };
    let members: Vec<PrincipalId> = body
        .member_ids
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect();

    match state
        .messaging
        .create_channel(ws, kind, body.name, body.topic, ctx.principal_id(), members)
        .await
    {
        Ok(ch) => (StatusCode::CREATED, Json(channel_json(&ch))).into_response(),
        Err(e) => map_err(e),
    }
}

async fn list_channels(
    State(state): State<MessagingHttpState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let ws: WorkspaceId = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state
        .messaging
        .list_channels(ws, ctx.principal_id())
        .await
    {
        Ok(list) => Json(list.iter().map(channel_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_channel(
    State(state): State<MessagingHttpState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id: ChannelId = match channel_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.messaging.get_channel(id).await {
        Ok(ch) => Json(channel_json(&ch)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn join_channel(
    State(state): State<MessagingHttpState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: ChannelId = match channel_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.messaging.join_channel(id, ctx.principal_id()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

async fn leave_channel(
    State(state): State<MessagingHttpState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: ChannelId = match channel_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.messaging.leave_channel(id, ctx.principal_id()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct SendMessageBody {
    content: String,
    thread_id: Option<String>,
    reply_to_id: Option<String>,
}

async fn send_message(
    State(state): State<MessagingHttpState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SendMessageBody>,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: ChannelId = match channel_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let thread_id = body.thread_id.and_then(|s| s.parse().ok());
    let reply_to_id = body.reply_to_id.and_then(|s| s.parse().ok());
    match state
        .messaging
        .send_message(id, ctx.principal_id(), body.content, thread_id, reply_to_id)
        .await
    {
        Ok(msg) => {
            if let Some(ref search) = state.search {
                if let Ok(ch) = state.messaging.get_channel(msg.channel_id).await {
                    search
                        .index_message(
                            msg.id,
                            msg.channel_id,
                            ch.workspace_id,
                            &msg.content,
                            msg.author_id,
                        )
                        .await;
                }
            }
            (StatusCode::CREATED, Json(message_json(&msg))).into_response()
        },
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct ListMessagesQuery {
    thread_id: Option<String>,
    before: Option<String>,
    limit: Option<u32>,
}

async fn list_messages(
    State(state): State<MessagingHttpState>,
    Path(channel_id): Path<String>,
    Query(q): Query<ListMessagesQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: ChannelId = match channel_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let thread_id = q.thread_id.and_then(|s| s.parse().ok());
    let before = q.before.and_then(|s| s.parse().ok());
    match state
        .messaging
        .list_messages(id, ctx.principal_id(), thread_id, before, q.limit.unwrap_or(50))
        .await
    {
        Ok(list) => Json(list.iter().map(message_json).collect::<Vec<_>>()).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct EditMessageBody {
    content: String,
}

async fn edit_message(
    State(state): State<MessagingHttpState>,
    Path(message_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<EditMessageBody>,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: MessageId = match message_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state
        .messaging
        .edit_message(id, ctx.principal_id(), body.content)
        .await
    {
        Ok(msg) => Json(message_json(&msg)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn delete_message(
    State(state): State<MessagingHttpState>,
    Path(message_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: MessageId = match message_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.messaging.delete_message(id, ctx.principal_id()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct ReactionBody {
    emoji: String,
}

async fn add_reaction(
    State(state): State<MessagingHttpState>,
    Path(message_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ReactionBody>,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: MessageId = match message_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state
        .messaging
        .add_reaction(id, ctx.principal_id(), body.emoji)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

async fn remove_reaction(
    State(state): State<MessagingHttpState>,
    Path(message_id): Path<String>,
    Query(body): Query<ReactionBody>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let id: MessageId = match message_id.parse() {
        Ok(i) => i,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state
        .messaging
        .remove_reaction(id, ctx.principal_id(), &body.emoji)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

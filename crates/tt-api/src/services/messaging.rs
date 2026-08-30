//! MessagingService gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use tt_auth::PrincipalResolver;
use tt_core::domain::ChannelKind;
use tt_core::events::EventPayload;
use tt_core::ids::{ChannelId, MessageId, PrincipalId, ThreadId, WorkspaceId};
use tt_messaging::{MessagingError, MessagingService};
use tt_realtime::{EventSubscriber, InMemoryRealtime};

use crate::interceptors::require_auth;
use crate::pb::common::v1::Id;
use crate::pb::messaging::v1::messaging_service_server::MessagingService as MessagingServiceTrait;
use crate::pb::messaging::v1::*;

pub struct MessagingServiceImpl {
    pub messaging: Arc<MessagingService>,
    pub resolver: Arc<PrincipalResolver>,
    pub realtime: Arc<InMemoryRealtime>,
}

impl MessagingServiceImpl {
    pub fn new(
        messaging: Arc<MessagingService>,
        resolver: Arc<PrincipalResolver>,
        realtime: Arc<InMemoryRealtime>,
    ) -> Self {
        Self {
            messaging,
            resolver,
            realtime,
        }
    }
}

fn map_err(e: MessagingError) -> Status {
    match e {
        MessagingError::NotFound => Status::not_found(e.to_string()),
        MessagingError::PermissionDenied(m) | MessagingError::Forbidden(m) => {
            Status::permission_denied(m)
        }
        MessagingError::InvalidArgument(m) => Status::invalid_argument(m),
        MessagingError::AlreadyExists => Status::already_exists("already exists"),
        MessagingError::Conflict(m) => Status::failed_precondition(m),
        MessagingError::Other(err) => Status::internal(err.to_string()),
    }
}

fn require_id(id: &Option<Id>, field: &str) -> Result<String, Status> {
    id.as_ref()
        .map(|i| i.value.clone())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Status::invalid_argument(format!("{field} required")))
}

fn parse_workspace(id: &Option<Id>) -> Result<WorkspaceId, Status> {
    require_id(id, "workspace_id")?
        .parse()
        .map_err(|_| Status::invalid_argument("invalid workspace_id"))
}

fn parse_channel(id: &Option<Id>) -> Result<ChannelId, Status> {
    require_id(id, "channel_id")?
        .parse()
        .map_err(|_| Status::invalid_argument("invalid channel_id"))
}

fn parse_message(id: &Option<Id>) -> Result<MessageId, Status> {
    require_id(id, "message_id")?
        .parse()
        .map_err(|_| Status::invalid_argument("invalid message_id"))
}

fn channel_to_pb(c: &tt_core::domain::Channel) -> Channel {
    Channel {
        id: Some(Id {
            value: c.id.to_string(),
        }),
        workspace_id: Some(Id {
            value: c.workspace_id.to_string(),
        }),
        kind: match c.kind {
            ChannelKind::Public => "public",
            ChannelKind::Private => "private",
            ChannelKind::Direct => "direct",
            ChannelKind::Thread => "private",
        }
        .into(),
        name: c.name.clone(),
        topic: c.topic.clone(),
        is_archived: c.is_archived,
        created_by: Some(Id {
            value: c.created_by.to_string(),
        }),
        created_at: Some(prost_types::Timestamp {
            seconds: c.created_at.timestamp(),
            nanos: c.created_at.timestamp_subsec_nanos() as i32,
        }),
        updated_at: Some(prost_types::Timestamp {
            seconds: c.updated_at.timestamp(),
            nanos: c.updated_at.timestamp_subsec_nanos() as i32,
        }),
    }
}

fn message_to_pb(m: &tt_core::domain::Message) -> Message {
    Message {
        id: Some(Id {
            value: m.id.to_string(),
        }),
        channel_id: Some(Id {
            value: m.channel_id.to_string(),
        }),
        thread_id: m.thread_id.map(|t| Id {
            value: t.to_string(),
        }),
        author_id: Some(Id {
            value: m.author_id.to_string(),
        }),
        content: if m.is_deleted {
            String::new()
        } else {
            m.content.clone()
        },
        reply_to_id: m.reply_to_id.map(|r| Id {
            value: r.to_string(),
        }),
        is_edited: m.is_edited,
        is_deleted: m.is_deleted,
        created_at: Some(prost_types::Timestamp {
            seconds: m.created_at.timestamp(),
            nanos: m.created_at.timestamp_subsec_nanos() as i32,
        }),
        updated_at: Some(prost_types::Timestamp {
            seconds: m.updated_at.timestamp(),
            nanos: m.updated_at.timestamp_subsec_nanos() as i32,
        }),
    }
}

#[tonic::async_trait]
impl MessagingServiceTrait for MessagingServiceImpl {
    async fn create_channel(
        &self,
        request: Request<CreateChannelRequest>,
    ) -> Result<Response<CreateChannelResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let req = request.into_inner();
        let workspace_id = parse_workspace(&req.workspace_id)?;
        let kind = match req.kind.as_str() {
            "public" => ChannelKind::Public,
            "private" => ChannelKind::Private,
            "direct" => ChannelKind::Direct,
            _ => {
                return Err(Status::invalid_argument(
                    "kind must be public|private|direct",
                ))
            }
        };
        let members: Vec<PrincipalId> = req
            .member_ids
            .iter()
            .filter_map(|id| id.value.parse().ok())
            .collect();

        let channel = self
            .messaging
            .create_channel(
                workspace_id,
                kind,
                req.name,
                req.topic,
                ctx.principal_id(),
                members,
            )
            .await
            .map_err(map_err)?;

        Ok(Response::new(CreateChannelResponse {
            channel: Some(channel_to_pb(&channel)),
        }))
    }

    async fn list_channels(
        &self,
        request: Request<ListChannelsRequest>,
    ) -> Result<Response<ListChannelsResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let workspace_id = parse_workspace(&request.into_inner().workspace_id)?;
        let channels = self
            .messaging
            .list_channels(workspace_id, ctx.principal_id())
            .await
            .map_err(map_err)?;
        Ok(Response::new(ListChannelsResponse {
            channels: channels.iter().map(channel_to_pb).collect(),
        }))
    }

    async fn get_channel(
        &self,
        request: Request<GetChannelRequest>,
    ) -> Result<Response<GetChannelResponse>, Status> {
        let _ctx = require_auth(&self.resolver, request.metadata()).await?;
        let channel_id = parse_channel(&request.into_inner().channel_id)?;
        let channel = self
            .messaging
            .get_channel(channel_id)
            .await
            .map_err(map_err)?;
        Ok(Response::new(GetChannelResponse {
            channel: Some(channel_to_pb(&channel)),
        }))
    }

    async fn join_channel(
        &self,
        request: Request<JoinChannelRequest>,
    ) -> Result<Response<JoinChannelResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let channel_id = parse_channel(&request.into_inner().channel_id)?;
        self.messaging
            .join_channel(channel_id, ctx.principal_id())
            .await
            .map_err(map_err)?;
        Ok(Response::new(JoinChannelResponse {}))
    }

    async fn leave_channel(
        &self,
        request: Request<LeaveChannelRequest>,
    ) -> Result<Response<LeaveChannelResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let channel_id = parse_channel(&request.into_inner().channel_id)?;
        self.messaging
            .leave_channel(channel_id, ctx.principal_id())
            .await
            .map_err(map_err)?;
        Ok(Response::new(LeaveChannelResponse {}))
    }

    async fn send_message(
        &self,
        request: Request<SendMessageRequest>,
    ) -> Result<Response<SendMessageResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let req = request.into_inner();
        let channel_id = parse_channel(&req.channel_id)?;
        let thread_id = req
            .thread_id
            .as_ref()
            .and_then(|id| id.value.parse::<ThreadId>().ok());
        let reply_to_id = req
            .reply_to_id
            .as_ref()
            .and_then(|id| id.value.parse::<MessageId>().ok());

        let message = self
            .messaging
            .send_message(
                channel_id,
                ctx.principal_id(),
                req.content,
                thread_id,
                reply_to_id,
            )
            .await
            .map_err(map_err)?;

        Ok(Response::new(SendMessageResponse {
            message: Some(message_to_pb(&message)),
        }))
    }

    async fn list_messages(
        &self,
        request: Request<ListMessagesRequest>,
    ) -> Result<Response<ListMessagesResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let req = request.into_inner();
        let channel_id = parse_channel(&req.channel_id)?;
        let thread_id = req
            .thread_id
            .as_ref()
            .and_then(|id| id.value.parse::<ThreadId>().ok());
        let before = req
            .before_id
            .as_ref()
            .and_then(|id| id.value.parse::<MessageId>().ok());
        let limit = if req.limit == 0 { 50 } else { req.limit };

        let messages = self
            .messaging
            .list_messages(channel_id, ctx.principal_id(), thread_id, before, limit)
            .await
            .map_err(map_err)?;

        let has_more = messages.len() as u32 >= limit;
        Ok(Response::new(ListMessagesResponse {
            messages: messages.iter().map(message_to_pb).collect(),
            has_more,
        }))
    }

    async fn edit_message(
        &self,
        request: Request<EditMessageRequest>,
    ) -> Result<Response<EditMessageResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let req = request.into_inner();
        let message_id = parse_message(&req.message_id)?;
        let message = self
            .messaging
            .edit_message(message_id, ctx.principal_id(), req.content)
            .await
            .map_err(map_err)?;
        Ok(Response::new(EditMessageResponse {
            message: Some(message_to_pb(&message)),
        }))
    }

    async fn delete_message(
        &self,
        request: Request<DeleteMessageRequest>,
    ) -> Result<Response<DeleteMessageResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let message_id = parse_message(&request.into_inner().message_id)?;
        self.messaging
            .delete_message(message_id, ctx.principal_id())
            .await
            .map_err(map_err)?;
        Ok(Response::new(DeleteMessageResponse {}))
    }

    async fn add_reaction(
        &self,
        request: Request<AddReactionRequest>,
    ) -> Result<Response<AddReactionResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let req = request.into_inner();
        let message_id = parse_message(&req.message_id)?;
        self.messaging
            .add_reaction(message_id, ctx.principal_id(), req.emoji)
            .await
            .map_err(map_err)?;
        Ok(Response::new(AddReactionResponse {}))
    }

    async fn remove_reaction(
        &self,
        request: Request<RemoveReactionRequest>,
    ) -> Result<Response<RemoveReactionResponse>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let req = request.into_inner();
        let message_id = parse_message(&req.message_id)?;
        self.messaging
            .remove_reaction(message_id, ctx.principal_id(), &req.emoji)
            .await
            .map_err(map_err)?;
        Ok(Response::new(RemoveReactionResponse {}))
    }

    type SubscribeChannelStream =
        Pin<Box<dyn Stream<Item = Result<ChannelEvent, Status>> + Send + 'static>>;

    async fn subscribe_channel(
        &self,
        request: Request<SubscribeChannelRequest>,
    ) -> Result<Response<Self::SubscribeChannelStream>, Status> {
        let ctx = require_auth(&self.resolver, request.metadata()).await?;
        let channel_id = parse_channel(&request.into_inner().channel_id)?;

        // Ensure caller can read the channel
        let _ = self
            .messaging
            .list_messages(channel_id, ctx.principal_id(), None, None, 1)
            .await
            .map_err(map_err)?;

        let channel = self
            .messaging
            .get_channel(channel_id)
            .await
            .map_err(map_err)?;

        let mut stream = self
            .realtime
            .subscribe_channel(channel.workspace_id, channel_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            while let Some(envelope) = stream.next().await {
                let event = match envelope.payload {
                    EventPayload::MessageCreated {
                        channel_id: cid,
                        message_id,
                        ..
                    } if cid == channel_id => {
                        // Client can refetch; we send id-only style events for stream
                        Some(ChannelEvent {
                            event: Some(channel_event::Event::MessageCreated(MessageCreated {
                                message: Some(Message {
                                    id: Some(Id {
                                        value: message_id.to_string(),
                                    }),
                                    channel_id: Some(Id {
                                        value: cid.to_string(),
                                    }),
                                    ..Default::default()
                                }),
                            })),
                        })
                    }
                    EventPayload::MessageUpdated {
                        channel_id: cid,
                        message_id,
                    } if cid == channel_id => Some(ChannelEvent {
                        event: Some(channel_event::Event::MessageUpdated(MessageUpdated {
                            message: Some(Message {
                                id: Some(Id {
                                    value: message_id.to_string(),
                                }),
                                channel_id: Some(Id {
                                    value: cid.to_string(),
                                }),
                                ..Default::default()
                            }),
                        })),
                    }),
                    EventPayload::MessageDeleted {
                        channel_id: cid,
                        message_id,
                    } if cid == channel_id => Some(ChannelEvent {
                        event: Some(channel_event::Event::MessageDeleted(MessageDeleted {
                            message_id: Some(Id {
                                value: message_id.to_string(),
                            }),
                        })),
                    }),
                    EventPayload::ReactionAdded {
                        channel_id: cid,
                        message_id,
                        principal_id,
                        emoji,
                    } if cid == channel_id => Some(ChannelEvent {
                        event: Some(channel_event::Event::ReactionAdded(ReactionAdded {
                            message_id: Some(Id {
                                value: message_id.to_string(),
                            }),
                            principal_id: Some(Id {
                                value: principal_id.to_string(),
                            }),
                            emoji,
                        })),
                    }),
                    EventPayload::ReactionRemoved {
                        channel_id: cid,
                        message_id,
                        principal_id,
                        emoji,
                    } if cid == channel_id => Some(ChannelEvent {
                        event: Some(channel_event::Event::ReactionRemoved(ReactionRemoved {
                            message_id: Some(Id {
                                value: message_id.to_string(),
                            }),
                            principal_id: Some(Id {
                                value: principal_id.to_string(),
                            }),
                            emoji,
                        })),
                    }),
                    _ => None,
                };

                if let Some(ev) = event {
                    if tx.send(Ok(ev)).await.is_err() {
                        break;
                    }
                }
            }
        });

        let out: Self::SubscribeChannelStream = Box::pin(ReceiverStream::new(rx));
        Ok(Response::new(out))
    }
}

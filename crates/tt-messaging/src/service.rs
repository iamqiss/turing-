//! Messaging application service.

use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::{Channel, ChannelKind, Message, Reaction};
use tt_core::events::{EventEnvelope, EventPayload};
use tt_core::ids::{ChannelId, MessageId, PrincipalId, ThreadId, WorkspaceId};
use tt_realtime::EventPublisher;
use uuid::Uuid;

use crate::error::{MessagingError, Result};
use crate::store::MessageStore;

pub struct MessagingService {
    store: Arc<dyn MessageStore>,
    events: Arc<dyn EventPublisher>,
}

impl MessagingService {
    pub fn new(store: Arc<dyn MessageStore>, events: Arc<dyn EventPublisher>) -> Self {
        Self { store, events }
    }

    pub async fn create_channel(
        &self,
        workspace_id: WorkspaceId,
        kind: ChannelKind,
        name: Option<String>,
        topic: Option<String>,
        created_by: PrincipalId,
        member_ids: Vec<PrincipalId>,
    ) -> Result<Channel> {
        if matches!(kind, ChannelKind::Public | ChannelKind::Private) && name.as_ref().map(|n| n.is_empty()).unwrap_or(true) {
            return Err(MessagingError::InvalidArgument(
                "name required for public/private channels".into(),
            ));
        }

        let now = Utc::now();
        let channel = Channel {
            id: ChannelId::new(),
            workspace_id,
            kind,
            name,
            topic,
            is_archived: false,
            created_by,
            created_at: now,
            updated_at: now,
        };

        self.store.create_channel(&channel, &member_ids).await?;

        let _ = self
            .events
            .publish_to_channel(
                workspace_id,
                channel.id,
                EventEnvelope {
                    id: Uuid::now_v7(),
                    workspace_id,
                    occurred_at: now,
                    payload: EventPayload::ChannelCreated {
                        channel_id: channel.id,
                    },
                },
            )
            .await;

        info!(channel_id = %channel.id, "channel created");
        Ok(channel)
    }


    /// Open or reuse a 1:1 DM between `caller` and `peer` in the workspace.
    pub async fn open_direct_message(
        &self,
        workspace_id: WorkspaceId,
        caller: PrincipalId,
        peer: PrincipalId,
    ) -> Result<Channel> {
        if caller == peer {
            return Err(MessagingError::InvalidArgument(
                "cannot open a DM with yourself".into(),
            ));
        }
        if let Some(existing) = self
            .store
            .find_direct_channel(workspace_id, caller, peer)
            .await?
        {
            return Ok(existing);
        }
        self.create_channel(
            workspace_id,
            ChannelKind::Direct,
            None,
            None,
            caller,
            vec![caller, peer],
        )
        .await
    }

    pub async fn list_channels(
        &self,
        workspace_id: WorkspaceId,
        principal_id: PrincipalId,
    ) -> Result<Vec<Channel>> {
        self.store
            .list_channels_for_principal(workspace_id, principal_id)
            .await
    }

    pub async fn get_channel(&self, id: ChannelId) -> Result<Channel> {
        self.store
            .get_channel(id)
            .await?
            .ok_or(MessagingError::NotFound)
    }

    pub async fn join_channel(
        &self,
        channel_id: ChannelId,
        principal_id: PrincipalId,
    ) -> Result<()> {
        let channel = self.get_channel(channel_id).await?;
        if channel.kind == ChannelKind::Private || channel.kind == ChannelKind::Direct {
            // Private/direct: must be invited (for v1 allow join if you know the id – tighten later)
        }
        self.store.add_member(channel_id, principal_id).await?;

        let _ = self
            .events
            .publish_to_channel(
                channel.workspace_id,
                channel_id,
                EventEnvelope {
                    id: Uuid::now_v7(),
                    workspace_id: channel.workspace_id,
                    occurred_at: Utc::now(),
                    payload: EventPayload::ChannelMemberJoined {
                        channel_id,
                        principal_id,
                    },
                },
            )
            .await;
        Ok(())
    }

    pub async fn leave_channel(
        &self,
        channel_id: ChannelId,
        principal_id: PrincipalId,
    ) -> Result<()> {
        let channel = self.get_channel(channel_id).await?;
        self.store.remove_member(channel_id, principal_id).await?;

        let _ = self
            .events
            .publish_to_channel(
                channel.workspace_id,
                channel_id,
                EventEnvelope {
                    id: Uuid::now_v7(),
                    workspace_id: channel.workspace_id,
                    occurred_at: Utc::now(),
                    payload: EventPayload::ChannelMemberLeft {
                        channel_id,
                        principal_id,
                    },
                },
            )
            .await;
        Ok(())
    }

    async fn ensure_can_write(
        &self,
        channel: &Channel,
        principal_id: PrincipalId,
    ) -> Result<()> {
        if channel.kind == ChannelKind::Public {
            // Auto-join public on write if not a member
            if !self.store.is_member(channel.id, principal_id).await? {
                self.store.add_member(channel.id, principal_id).await?;
            }
            return Ok(());
        }
        if !self.store.is_member(channel.id, principal_id).await? {
            return Err(MessagingError::PermissionDenied(
                "not a member of this channel".into(),
            ));
        }
        Ok(())
    }

    pub async fn send_message(
        &self,
        channel_id: ChannelId,
        author_id: PrincipalId,
        content: String,
        thread_id: Option<ThreadId>,
        reply_to_id: Option<MessageId>,
    ) -> Result<Message> {
        if content.trim().is_empty() {
            return Err(MessagingError::InvalidArgument("content required".into()));
        }
        let channel = self.get_channel(channel_id).await?;
        self.ensure_can_write(&channel, author_id).await?;

        let now = Utc::now();
        let msg = Message {
            id: MessageId::new(),
            channel_id,
            thread_id,
            author_id,
            content,
            reply_to_id,
            is_edited: false,
            is_deleted: false,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_message(&msg).await?;

        let _ = self
            .events
            .publish_to_channel(
                channel.workspace_id,
                channel_id,
                EventEnvelope {
                    id: Uuid::now_v7(),
                    workspace_id: channel.workspace_id,
                    occurred_at: now,
                    payload: EventPayload::MessageCreated {
                        channel_id,
                        message_id: msg.id,
                        author_id,
                    },
                },
            )
            .await;

        Ok(msg)
    }

    pub async fn list_messages(
        &self,
        channel_id: ChannelId,
        principal_id: PrincipalId,
        thread_id: Option<ThreadId>,
        before: Option<MessageId>,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let channel = self.get_channel(channel_id).await?;
        if channel.kind != ChannelKind::Public
            && !self.store.is_member(channel_id, principal_id).await?
        {
            return Err(MessagingError::PermissionDenied(
                "not a member of this channel".into(),
            ));
        }
        self.store
            .list_messages(channel_id, thread_id, before, limit)
            .await
    }

    pub async fn edit_message(
        &self,
        message_id: MessageId,
        principal_id: PrincipalId,
        content: String,
    ) -> Result<Message> {
        let msg = self
            .store
            .get_message(message_id)
            .await?
            .ok_or(MessagingError::NotFound)?;
        if msg.author_id != principal_id {
            return Err(MessagingError::PermissionDenied(
                "only the author can edit".into(),
            ));
        }
        if content.trim().is_empty() {
            return Err(MessagingError::InvalidArgument("content required".into()));
        }
        self.store
            .update_message_content(message_id, &content)
            .await?;
        let mut updated = msg;
        updated.content = content;
        updated.is_edited = true;
        updated.updated_at = Utc::now();

        if let Ok(channel) = self.get_channel(updated.channel_id).await {
            let _ = self
                .events
                .publish_to_channel(
                    channel.workspace_id,
                    updated.channel_id,
                    EventEnvelope {
                        id: Uuid::now_v7(),
                        workspace_id: channel.workspace_id,
                        occurred_at: updated.updated_at,
                        payload: EventPayload::MessageUpdated {
                            channel_id: updated.channel_id,
                            message_id,
                        },
                    },
                )
                .await;
        }
        Ok(updated)
    }

    pub async fn delete_message(
        &self,
        message_id: MessageId,
        principal_id: PrincipalId,
    ) -> Result<()> {
        let msg = self
            .store
            .get_message(message_id)
            .await?
            .ok_or(MessagingError::NotFound)?;
        if msg.author_id != principal_id {
            return Err(MessagingError::PermissionDenied(
                "only the author can delete".into(),
            ));
        }
        self.store.soft_delete_message(message_id).await?;

        if let Ok(channel) = self.get_channel(msg.channel_id).await {
            let _ = self
                .events
                .publish_to_channel(
                    channel.workspace_id,
                    msg.channel_id,
                    EventEnvelope {
                        id: Uuid::now_v7(),
                        workspace_id: channel.workspace_id,
                        occurred_at: Utc::now(),
                        payload: EventPayload::MessageDeleted {
                            channel_id: msg.channel_id,
                            message_id,
                        },
                    },
                )
                .await;
        }
        Ok(())
    }

    pub async fn add_reaction(
        &self,
        message_id: MessageId,
        principal_id: PrincipalId,
        emoji: String,
    ) -> Result<()> {
        if emoji.trim().is_empty() {
            return Err(MessagingError::InvalidArgument("emoji required".into()));
        }
        let msg = self
            .store
            .get_message(message_id)
            .await?
            .ok_or(MessagingError::NotFound)?;
        let channel = self.get_channel(msg.channel_id).await?;
        self.ensure_can_write(&channel, principal_id).await?;

        let reaction = Reaction {
            message_id,
            principal_id,
            emoji: emoji.clone(),
            created_at: Utc::now(),
        };
        self.store.add_reaction(&reaction).await?;

        let _ = self
            .events
            .publish_to_channel(
                channel.workspace_id,
                msg.channel_id,
                EventEnvelope {
                    id: Uuid::now_v7(),
                    workspace_id: channel.workspace_id,
                    occurred_at: reaction.created_at,
                    payload: EventPayload::ReactionAdded {
                        channel_id: msg.channel_id,
                        message_id,
                        principal_id,
                        emoji,
                    },
                },
            )
            .await;
        Ok(())
    }

    pub async fn remove_reaction(
        &self,
        message_id: MessageId,
        principal_id: PrincipalId,
        emoji: &str,
    ) -> Result<()> {
        let msg = self
            .store
            .get_message(message_id)
            .await?
            .ok_or(MessagingError::NotFound)?;
        self.store
            .remove_reaction(message_id, principal_id, emoji)
            .await?;

        if let Ok(channel) = self.get_channel(msg.channel_id).await {
            let _ = self
                .events
                .publish_to_channel(
                    channel.workspace_id,
                    msg.channel_id,
                    EventEnvelope {
                        id: Uuid::now_v7(),
                        workspace_id: channel.workspace_id,
                        occurred_at: Utc::now(),
                        payload: EventPayload::ReactionRemoved {
                            channel_id: msg.channel_id,
                            message_id,
                            principal_id,
                            emoji: emoji.to_string(),
                        },
                    },
                )
                .await;
        }
        Ok(())
    }
}

//! Event subscriber trait.

use async_trait::async_trait;
use tt_core::events::EventEnvelope;
use tt_core::ids::{ChannelId, WorkspaceId};

#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Subscribe to all events in a workspace.
    async fn subscribe_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Box<dyn EventStream>>;

    /// Subscribe to a single channel.
    async fn subscribe_channel(
        &self,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
    ) -> anyhow::Result<Box<dyn EventStream>>;
}

#[async_trait]
pub trait EventStream: Send {
    async fn next(&mut self) -> Option<EventEnvelope>;
}

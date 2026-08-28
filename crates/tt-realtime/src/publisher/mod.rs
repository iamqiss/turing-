//! Event publisher trait – the only thing domain code talks to.

use async_trait::async_trait;
use tt_core::events::EventEnvelope;
use tt_core::ids::WorkspaceId;

#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish an event to all subscribers of the given workspace (and optionally channel).
    async fn publish(&self, event: EventEnvelope) -> anyhow::Result<()>;

    /// Publish to a specific channel within a workspace.
    async fn publish_to_channel(
        &self,
        workspace_id: WorkspaceId,
        channel_id: tt_core::ids::ChannelId,
        event: EventEnvelope,
    ) -> anyhow::Result<()>;
}

//! In-process fanout via tokio broadcast channels.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{broadcast, RwLock};
use tt_core::events::EventEnvelope;
use tt_core::ids::{ChannelId, WorkspaceId};

use crate::publisher::EventPublisher;
use crate::subscriber::{EventStream, EventSubscriber};

const CAPACITY: usize = 1024;

#[derive(Clone, Default)]
pub struct InMemoryRealtime {
    /// workspace -> broadcast
    workspaces: Arc<RwLock<HashMap<String, broadcast::Sender<EventEnvelope>>>>,
    /// channel -> broadcast
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<EventEnvelope>>>>,
}

impl InMemoryRealtime {
    pub fn new() -> Self {
        Self::default()
    }

    async fn workspace_tx(&self, id: WorkspaceId) -> broadcast::Sender<EventEnvelope> {
        let key = id.to_string();
        let mut map = self.workspaces.write().await;
        map.entry(key)
            .or_insert_with(|| broadcast::channel(CAPACITY).0)
            .clone()
    }

    async fn channel_tx(&self, id: ChannelId) -> broadcast::Sender<EventEnvelope> {
        let key = id.to_string();
        let mut map = self.channels.write().await;
        map.entry(key)
            .or_insert_with(|| broadcast::channel(CAPACITY).0)
            .clone()
    }
}

#[async_trait]
impl EventPublisher for InMemoryRealtime {
    async fn publish(&self, event: EventEnvelope) -> anyhow::Result<()> {
        let tx = self.workspace_tx(event.workspace_id).await;
        let _ = tx.send(event);
        Ok(())
    }

    async fn publish_to_channel(
        &self,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        event: EventEnvelope,
    ) -> anyhow::Result<()> {
        let _ = self.workspace_tx(workspace_id).await.send(event.clone());
        let _ = self.channel_tx(channel_id).await.send(event);
        Ok(())
    }
}

#[async_trait]
impl EventSubscriber for InMemoryRealtime {
    async fn subscribe_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Box<dyn EventStream>> {
        let tx = self.workspace_tx(workspace_id).await;
        let rx = tx.subscribe();
        Ok(Box::new(BroadcastStream { rx }))
    }

    async fn subscribe_channel(
        &self,
        _workspace_id: WorkspaceId,
        channel_id: ChannelId,
    ) -> anyhow::Result<Box<dyn EventStream>> {
        let tx = self.channel_tx(channel_id).await;
        let rx = tx.subscribe();
        Ok(Box::new(BroadcastStream { rx }))
    }
}

struct BroadcastStream {
    rx: broadcast::Receiver<EventEnvelope>,
}

#[async_trait]
impl EventStream for BroadcastStream {
    async fn next(&mut self) -> Option<EventEnvelope> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

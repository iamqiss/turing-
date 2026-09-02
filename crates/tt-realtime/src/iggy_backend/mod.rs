//! Apache Iggy fanout backend (HTTP transport).
//!
//! Enable with `realtime_backend = "iggy"` and a reachable Iggy HTTP API
//! (`iggy_url`, default http://127.0.0.1:3000).
//!
//! Architecture:
//! - Always mirrors into a local [`InMemoryRealtime`] so same-process
//!   subscribers (SSE/WS) keep working without an Iggy consumer loop.
//! - When enabled, also publishes JSON envelopes to Iggy stream/topic so
//!   other nodes / workers can poll via consumer groups.
//!
//! Disable anytime: set `realtime_backend = "memory"` (or `"redis"`).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::json;
use tracing::{debug, info, warn};
use tt_core::events::EventEnvelope;
use tt_core::ids::{ChannelId, WorkspaceId};

use crate::in_memory::InMemoryRealtime;
use crate::publisher::EventPublisher;

#[derive(Clone)]
pub struct IggyConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub stream: String,
    pub topic: String,
}

impl IggyConfig {
    pub fn from_parts(
        base_url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
        stream: impl Into<String>,
        topic: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            username: username.into(),
            password: password.into(),
            stream: stream.into(),
            topic: topic.into(),
        }
    }
}

/// Dual-write publisher: local broadcast + Iggy HTTP.
pub struct IggyRealtime {
    local: Arc<InMemoryRealtime>,
    http: Client,
    cfg: IggyConfig,
    /// Token from login (Basic or PAT-style); refreshed lazily if empty.
    token: tokio::sync::RwLock<Option<String>>,
    enabled: bool,
}

impl IggyRealtime {
    /// Connect, login, ensure stream/topic. On hard failure returns Err —
    /// caller should fall back to memory-only.
    pub async fn connect(local: Arc<InMemoryRealtime>, cfg: IggyConfig) -> anyhow::Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let this = Self {
            local,
            http,
            cfg,
            token: tokio::sync::RwLock::new(None),
            enabled: true,
        };
        this.login().await?;
        this.ensure_topology().await?;
        info!(
            stream = %this.cfg.stream,
            topic = %this.cfg.topic,
            url = %this.cfg.base_url,
            "iggy realtime connected"
        );
        Ok(this)
    }

    /// Memory-only shim (Iggy disabled / failed).
    pub fn memory_only(local: Arc<InMemoryRealtime>) -> Self {
        Self {
            local,
            http: Client::new(),
            cfg: IggyConfig::from_parts("", "", "", "tt", "events"),
            token: tokio::sync::RwLock::new(None),
            enabled: false,
        }
    }

    pub fn local(&self) -> Arc<InMemoryRealtime> {
        self.local.clone()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    async fn login(&self) -> anyhow::Result<()> {
        if !self.enabled || self.cfg.base_url.is_empty() {
            return Ok(());
        }
        // Iggy HTTP login — POST /users/login
        let url = format!("{}/users/login", self.cfg.base_url);
        let res = self
            .http
            .post(&url)
            .json(&json!({
                "username": self.cfg.username,
                "password": self.cfg.password,
            }))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().await.unwrap_or(json!({}));
                // tokens appear as access_token / token depending on version
                let tok = body
                    .get("access_token")
                    .or_else(|| body.get("token"))
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        body.get("access_token")
                            .and_then(|v| v.get("token"))
                            .and_then(|v| v.as_str())
                    })
                    .map(|s| s.to_string());
                if let Some(t) = tok {
                    *self.token.write().await = Some(t);
                    debug!("iggy login ok");
                } else {
                    // Some deployments accept basic auth without token body
                    warn!("iggy login: no token in response; will use basic auth fallback");
                    let basic = base64::engine::general_purpose::STANDARD.encode(
                        format!("{}:{}", self.cfg.username, self.cfg.password),
                    );
                    *self.token.write().await = Some(format!("Basic {basic}"));
                }
                Ok(())
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                // Fallback: try without auth (dev default-credentials already bound)
                warn!(%status, %text, "iggy login non-success; continuing with basic auth header");
                let basic = base64::engine::general_purpose::STANDARD.encode(
                    format!("{}:{}", self.cfg.username, self.cfg.password),
                );
                *self.token.write().await = Some(format!("Basic {basic}"));
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("iggy login request failed: {e}")),
        }
    }

    async fn auth_header(&self) -> Option<String> {
        self.token.read().await.clone()
    }

    async fn ensure_topology(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        // Create stream (ignore already-exists)
        let stream_url = format!("{}/streams", self.cfg.base_url);
        let mut req = self.http.post(&stream_url).json(&json!({
            "name": self.cfg.stream,
        }));
        if let Some(a) = self.auth_header().await {
            if a.starts_with("Basic ") {
                req = req.header("Authorization", a);
            } else {
                req = req.bearer_auth(a);
            }
        }
        let _ = req.send().await;

        // Create topic under stream
        let topic_url = format!(
            "{}/streams/{}/topics",
            self.cfg.base_url,
            urlencoding_path(&self.cfg.stream)
        );
        let mut req = self.http.post(&topic_url).json(&json!({
            "name": self.cfg.topic,
            "partitions_count": 8,
            "compression_algorithm": "none",
            "message_expiry": 0u64,
            "max_topic_size": 0u64,
        }));
        if let Some(a) = self.auth_header().await {
            if a.starts_with("Basic ") {
                req = req.header("Authorization", a);
            } else {
                req = req.bearer_auth(a);
            }
        }
        match req.send().await {
            Ok(r) if r.status().is_success() || r.status().as_u16() == 409 => Ok(()),
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                // Non-fatal: topic may already exist under different error code
                warn!(%status, %text, "iggy ensure topic: continuing");
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "iggy ensure topic request failed");
                Ok(())
            }
        }
    }

    async fn publish_iggy(&self, event: &EventEnvelope, partition_key: Option<&str>) {
        if !self.enabled {
            return;
        }
        let payload = match serde_json::to_vec(event) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "iggy serialize event");
                return;
            }
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&payload);

        // Prefer messages_key partitioning when channel-scoped
        let body = if let Some(key) = partition_key {
            json!({
                "partitioning": { "kind": "messages_key", "value": key },
                "messages": [{ "payload": b64 }]
            })
        } else {
            json!({
                "partitioning": { "kind": "balanced" },
                "messages": [{ "payload": b64 }]
            })
        };

        let url = format!(
            "{}/streams/{}/topics/{}/messages",
            self.cfg.base_url,
            urlencoding_path(&self.cfg.stream),
            urlencoding_path(&self.cfg.topic)
        );
        let mut req = self.http.post(&url).json(&body);
        if let Some(a) = self.auth_header().await {
            if a.starts_with("Basic ") {
                req = req.header("Authorization", a);
            } else {
                req = req.bearer_auth(a);
            }
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => {
                debug!(workspace = %event.workspace_id, "iggy published");
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                warn!(%status, %text, "iggy publish non-success");
            }
            Err(e) => warn!(error = %e, "iggy publish failed"),
        }
    }
}

fn urlencoding_path(s: &str) -> String {
    // stream/topic names are path-safe identifiers in practice
    s.replace('/', "%2F")
}

#[async_trait]
impl EventPublisher for IggyRealtime {
    async fn publish(&self, event: EventEnvelope) -> anyhow::Result<()> {
        // Local first — never block UX on remote bus
        self.local.publish(event.clone()).await?;
        self.publish_iggy(&event, None).await;
        Ok(())
    }

    async fn publish_to_channel(
        &self,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        event: EventEnvelope,
    ) -> anyhow::Result<()> {
        self.local
            .publish_to_channel(workspace_id, channel_id, event.clone())
            .await?;
        let key = channel_id.to_string();
        self.publish_iggy(&event, Some(&key)).await;
        Ok(())
    }
}

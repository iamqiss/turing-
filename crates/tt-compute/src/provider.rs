//! Hypervisor driver trait — control plane → tt-node.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tt_core::domain::{HypervisorDriver, ResourceSpec};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCommand {
    pub instance_id: String,
    pub driver: HypervisorDriver,
    pub action: NodeAction,
    pub resources: ResourceSpec,
    pub image_slug: Option<String>,
    /// Host TAP name (e.g. tt-tap-a1b2)
    pub tap_device: Option<String>,
    pub nat_public_ip: Option<String>,
    pub nat_port_start: Option<u16>,
    pub nat_port_end: Option<u16>,
    /// Deploy context
    pub git_clone_url: Option<String>,
    pub git_sha: Option<String>,
    pub deploy_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeAction {
    Boot,
    Stop,
    Destroy,
    Suspend,
}

#[async_trait]
pub trait HypervisorProvider: Send + Sync {
    async fn dispatch(&self, node_endpoint: &str, cmd: NodeCommand) -> Result<(), ProviderError>;
}

pub struct NullProvider;

#[async_trait]
impl HypervisorProvider for NullProvider {
    async fn dispatch(&self, _node_endpoint: &str, cmd: NodeCommand) -> Result<(), ProviderError> {
        tracing::info!(
            instance = %cmd.instance_id,
            action = ?cmd.action,
            driver = cmd.driver.as_str(),
            "null provider: command accepted"
        );
        Ok(())
    }
}

pub struct HttpProvider {
    client: reqwest::Client,
}

impl HttpProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
        }
    }
}

impl Default for HttpProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HypervisorProvider for HttpProvider {
    async fn dispatch(&self, node_endpoint: &str, cmd: NodeCommand) -> Result<(), ProviderError> {
        let base = node_endpoint.trim_end_matches('/');
        let url = format!("{base}/v1/command");
        let body = serde_json::json!({
            "instance_id": cmd.instance_id,
            "driver": cmd.driver.as_str(),
            "action": match cmd.action {
                NodeAction::Boot => "boot",
                NodeAction::Stop => "stop",
                NodeAction::Destroy => "destroy",
                NodeAction::Suspend => "suspend",
            },
            "resources": {
                "vcpu": cmd.resources.vcpu,
                "memory_mb": cmd.resources.memory_mb,
                "disk_gb": cmd.resources.disk_gb,
            },
            "image_slug": cmd.image_slug,
            "tap_device": cmd.tap_device,
            "nat_public_ip": cmd.nat_public_ip,
            "nat_port_start": cmd.nat_port_start,
            "nat_port_end": cmd.nat_port_end,
            "git_clone_url": cmd.git_clone_url,
            "git_sha": cmd.git_sha,
            "deploy_token": cmd.deploy_token,
        });
        let res = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Message(format!("tt-node unreachable: {e}")))?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(ProviderError::Message(format!("tt-node {status}: {text}")));
        }
        tracing::info!(instance = %cmd.instance_id, %url, "dispatched to tt-node");
        Ok(())
    }
}

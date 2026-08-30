pub mod firecracker;
pub mod cloud_hypervisor;
pub mod kata;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceSpec {
    #[serde(default = "one")]
    pub vcpu: u32,
    #[serde(default = "mem512")]
    pub memory_mb: u32,
    #[serde(default = "disk10")]
    pub disk_gb: u32,
}

fn one() -> u32 {
    1
}
fn mem512() -> u32 {
    512
}
fn disk10() -> u32 {
    10
}

#[derive(Debug, Clone, Deserialize)]
pub struct DriverRequest {
    pub instance_id: String,
    pub driver: String,
    pub action: String,
    pub resources: Option<ResourceSpec>,
    pub image_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverResult {
    pub accepted: bool,
    pub instance_id: String,
    pub driver: String,
    pub action: String,
    pub pid: Option<u32>,
    pub socket: Option<String>,
    pub note: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("unsupported action: {0}")]
    UnsupportedAction(String),
    #[error("missing asset: {0}")]
    MissingAsset(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

#[async_trait]
pub trait Driver: Send + Sync {
    fn name(&self) -> &'static str;
    async fn boot(
        &self,
        instance_id: &str,
        resources: &ResourceSpec,
        image_slug: Option<&str>,
    ) -> Result<DriverResult, DriverError>;
    async fn stop(&self, instance_id: &str) -> Result<DriverResult, DriverError>;
    async fn destroy(&self, instance_id: &str) -> Result<DriverResult, DriverError>;
    async fn suspend(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        Err(DriverError::UnsupportedAction("suspend".into()))
    }
}

pub async fn dispatch(
    fc: &firecracker::FirecrackerDriver,
    ch: &cloud_hypervisor::CloudHypervisorDriver,
    kata: &kata::KataDriver,
    req: DriverRequest,
) -> Result<DriverResult, DriverError> {
    let resources = req.resources.unwrap_or(ResourceSpec {
        vcpu: 1,
        memory_mb: 512,
        disk_gb: 10,
    });
    let slug = req.image_slug.as_deref();
    let driver: &dyn Driver = match req.driver.as_str() {
        "firecracker" | "serverless" => fc,
        "cloud_hypervisor" | "vm" => ch,
        "kata" => kata,
        other => {
            return Err(DriverError::Other(format!("unknown driver: {other}")));
        }
    };
    match req.action.as_str() {
        "boot" => driver.boot(&req.instance_id, &resources, slug).await,
        "stop" => driver.stop(&req.instance_id).await,
        "destroy" => driver.destroy(&req.instance_id).await,
        "suspend" => driver.suspend(&req.instance_id).await,
        other => Err(DriverError::UnsupportedAction(other.into())),
    }
}

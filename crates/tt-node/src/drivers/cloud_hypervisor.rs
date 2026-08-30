//! Cloud Hypervisor driver — stub with instance dir lifecycle (full CH next).

use std::path::PathBuf;

use async_trait::async_trait;
use tokio::fs;
use tracing::info;

use super::{Driver, DriverError, DriverResult, ResourceSpec};

#[derive(Clone)]
pub struct CloudHypervisorDriver {
    data_dir: PathBuf,
}

impl CloudHypervisorDriver {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn instance_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("instances").join(id)
    }
}

#[async_trait]
impl Driver for CloudHypervisorDriver {
    fn name(&self) -> &'static str {
        "cloud_hypervisor"
    }

    async fn boot(
        &self,
        instance_id: &str,
        resources: &ResourceSpec,
        _image_slug: Option<&str>,
    ) -> Result<DriverResult, DriverError> {
        let dir = self.instance_dir(instance_id);
        fs::create_dir_all(&dir).await?;
        fs::write(
            dir.join("status.json"),
            format!(
                r#"{{"status":"pending_ch","vcpu":{},"memory_mb":{}}}"#,
                resources.vcpu, resources.memory_mb
            ),
        )
        .await?;
        info!(%instance_id, "cloud-hypervisor boot stub");
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "cloud_hypervisor".into(),
            action: "boot".into(),
            pid: None,
            socket: None,
            note: "stub: install cloud-hypervisor; C3 wires full boot".into(),
        })
    }

    async fn stop(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "cloud_hypervisor".into(),
            action: "stop".into(),
            pid: None,
            socket: None,
            note: "stub stop".into(),
        })
    }

    async fn destroy(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        let dir = self.instance_dir(instance_id);
        if dir.exists() {
            fs::remove_dir_all(dir).await?;
        }
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "cloud_hypervisor".into(),
            action: "destroy".into(),
            pid: None,
            socket: None,
            note: "removed".into(),
        })
    }
}

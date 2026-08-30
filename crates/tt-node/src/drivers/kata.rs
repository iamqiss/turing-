//! Kata Containers driver — stub.

use async_trait::async_trait;
use tracing::info;

use super::{Driver, DriverError, DriverResult, ResourceSpec};

#[derive(Clone, Default)]
pub struct KataDriver;

#[async_trait]
impl Driver for KataDriver {
    fn name(&self) -> &'static str {
        "kata"
    }

    async fn boot(
        &self,
        instance_id: &str,
        resources: &ResourceSpec,
        _image_slug: Option<&str>,
    ) -> Result<DriverResult, DriverError> {
        info!(%instance_id, vcpu = resources.vcpu, "kata boot stub");
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "kata".into(),
            action: "boot".into(),
            pid: None,
            socket: None,
            note: "stub: kata-runtime path later".into(),
        })
    }

    async fn stop(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "kata".into(),
            action: "stop".into(),
            pid: None,
            socket: None,
            note: "stub".into(),
        })
    }

    async fn destroy(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "kata".into(),
            action: "destroy".into(),
            pid: None,
            socket: None,
            note: "stub".into(),
        })
    }
}

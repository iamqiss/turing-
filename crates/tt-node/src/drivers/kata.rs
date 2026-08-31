use async_trait::async_trait;
use tracing::info;
use super::{Driver, DriverError, DriverRequest, DriverResult, ResourceSpec};

#[derive(Clone, Default)]
pub struct KataDriver;

#[async_trait]
impl Driver for KataDriver {
    fn name(&self) -> &'static str { "kata" }

    async fn boot(&self, req: &DriverRequest, resources: &ResourceSpec) -> Result<DriverResult, DriverError> {
        info!(instance = %req.instance_id, vcpu = resources.vcpu, "kata boot stub");
        Ok(DriverResult {
            accepted: true,
            instance_id: req.instance_id.clone(),
            driver: "kata".into(),
            action: "boot".into(),
            pid: None, socket: None,
            tap_device: req.tap_device.clone(),
            note: "stub".into(),
        })
    }

    async fn stop(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "kata".into(),
            action: "stop".into(),
            pid: None, socket: None, tap_device: None,
            note: "stub".into(),
        })
    }

    async fn destroy(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "kata".into(),
            action: "destroy".into(),
            pid: None, socket: None, tap_device: None,
            note: "stub".into(),
        })
    }
}

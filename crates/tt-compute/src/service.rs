use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::*;
use tt_core::ids::*;
use tt_net::IpAllocator;

use crate::error::{ComputeError, Result};
use crate::provider::{HypervisorProvider, NodeAction, NodeCommand};
use crate::store::ComputeStore;

pub struct ComputeService {
    store: Arc<dyn ComputeStore>,
    net: Arc<dyn IpAllocator>,
    provider: Arc<dyn HypervisorProvider>,
}

impl ComputeService {
    pub fn new(
        store: Arc<dyn ComputeStore>,
        net: Arc<dyn IpAllocator>,
        provider: Arc<dyn HypervisorProvider>,
    ) -> Self {
        Self {
            store,
            net,
            provider,
        }
    }

    pub async fn register_node(
        &self,
        name: String,
        region: RegionCode,
        physical_cores: u32,
        memory_mb: u32,
        reserved_cores: u32,
        reserved_memory_mb: u32,
        overcommit_ratio: f32,
    ) -> Result<ComputeNode> {
        if physical_cores == 0 || memory_mb == 0 {
            return Err(ComputeError::InvalidArgument(
                "physical_cores and memory_mb required".into(),
            ));
        }
        if reserved_cores >= physical_cores {
            return Err(ComputeError::InvalidArgument(
                "reserved_cores must be < physical_cores".into(),
            ));
        }
        let node = ComputeNode {
            id: NodeId::new(),
            name,
            region,
            status: NodeStatus::Registering,
            physical_cores,
            memory_mb,
            reserved_cores,
            reserved_memory_mb,
            overcommit_ratio: if overcommit_ratio <= 0.0 {
                4.0
            } else {
                overcommit_ratio
            },
            agent_endpoint: None,
            last_heartbeat: None,
            created_at: Utc::now(),
        };
        self.store.insert_node(&node).await?;
        info!(
            node = %node.id,
            cores = physical_cores,
            sellable = node.sellable_cores(),
            cap_1vcpu = node.capacity_1vcpu(),
            "compute node registered"
        );
        Ok(node)
    }

    pub async fn list_nodes(&self, region: Option<RegionCode>) -> Result<Vec<ComputeNode>> {
        self.store.list_nodes(region).await
    }

    pub async fn heartbeat(&self, node_id: NodeId, endpoint: &str) -> Result<()> {
        self.store.heartbeat(node_id, endpoint).await
    }

    pub async fn create_workload(
        &self,
        org_id: OrgId,
        created_by: PrincipalId,
        name: String,
        shape: WorkloadShape,
        resources: ResourceSpec,
        region: RegionCode,
        image_template_id: Option<ImageTemplateId>,
        repository_id: Option<RepositoryId>,
        git_ref: Option<String>,
    ) -> Result<Workload> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(ComputeError::InvalidArgument("name required".into()));
        }
        if resources.vcpu == 0 {
            return Err(ComputeError::InvalidArgument("vcpu must be >= 1".into()));
        }

        let (max_wl, max_vcpu, _) = self.store.get_quota(org_id).await?;
        let used = self.store.count_org_workloads(org_id).await?;
        if used >= max_wl {
            return Err(ComputeError::QuotaExceeded(format!(
                "max workloads {max_wl}"
            )));
        }
        // rough: sum not fully tracked yet — gate on single workload size
        if resources.vcpu > max_vcpu {
            return Err(ComputeError::QuotaExceeded(format!(
                "vcpu {0} > org max {max_vcpu}",
                resources.vcpu
            )));
        }

        let now = Utc::now();
        let wl = Workload {
            id: WorkloadId::new(),
            org_id,
            name,
            shape,
            status: WorkloadStatus::Pending,
            resources,
            image_template_id,
            repository_id,
            git_ref,
            git_sha: None,
            region,
            created_by,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_workload(&wl).await?;
        info!(workload = %wl.id, shape = wl.shape.as_str(), "workload created");
        Ok(wl)
    }

    pub async fn list_workloads(&self, org_id: OrgId) -> Result<Vec<Workload>> {
        self.store.list_workloads(org_id).await
    }

    pub async fn get_workload(&self, id: WorkloadId) -> Result<Workload> {
        self.store
            .get_workload(id)
            .await?
            .ok_or(ComputeError::NotFound)
    }

    /// Schedule onto a ready node in region, allocate NAT, dispatch boot (or null).
    pub async fn start(&self, workload_id: WorkloadId) -> Result<Instance> {
        let wl = self.get_workload(workload_id).await?;
        if matches!(
            wl.status,
            WorkloadStatus::Running | WorkloadStatus::Destroyed
        ) {
            return Err(ComputeError::Conflict(format!(
                "cannot start from status {}",
                wl.status.as_str()
            )));
        }

        let nodes = self.store.list_nodes(Some(wl.region)).await?;
        let node = nodes
            .into_iter()
            .find(|n| n.status == NodeStatus::Ready)
            .ok_or_else(|| {
                ComputeError::NoCapacity(format!(
                    "no ready node in {}",
                    wl.region.as_str()
                ))
            })?;

        let now = Utc::now();
        let mut inst = Instance {
            id: InstanceId::new(),
            workload_id,
            node_id: node.id,
            driver: wl.shape.preferred_driver(),
            status: WorkloadStatus::Scheduled,
            private_ip: None,
            ipv6: None,
            nat_public_ip: None,
            nat_port_start: None,
            nat_port_end: None,
            dedicated_ipv4: None,
            hypervisor_id: None,
            created_at: now,
            updated_at: now,
        };

        // NAT allocation (CloudRat shared model)
        match self.net.allocate_nat(wl.region, inst.id).await {
            Ok(nat) => {
                inst.nat_public_ip = Some(nat.public_ip);
                inst.nat_port_start = Some(nat.port_start);
                inst.nat_port_end = Some(nat.port_end);
                inst.ipv6 = nat.ipv6;
            }
            Err(e) => {
                tracing::warn!(error = %e, "NAT allocate failed — continuing without public map");
            }
        }

        self.store.insert_instance(&inst).await?;

        if let Some(ref endpoint) = node.agent_endpoint {
            let cmd = NodeCommand {
                instance_id: inst.id.to_string(),
                driver: inst.driver,
                action: NodeAction::Boot,
                resources: wl.resources.clone(),
                image_slug: None,
            };
            if let Err(e) = self.provider.dispatch(endpoint, cmd).await {
                // Lab-friendly: record instance running intent even if agent is down.
                tracing::warn!(error = %e, "tt-node dispatch failed; instance recorded as running");
            }
        }

        inst.status = WorkloadStatus::Running;
        inst.updated_at = Utc::now();
        self.store.update_instance(&inst).await?;
        self.store
            .update_workload_status(workload_id, WorkloadStatus::Running)
            .await?;

        info!(
            instance = %inst.id,
            node = %node.id,
            driver = inst.driver.as_str(),
            "instance started"
        );
        Ok(inst)
    }

    pub async fn stop(&self, workload_id: WorkloadId) -> Result<()> {
        let instances = self.store.list_instances_for_workload(workload_id).await?;
        for mut inst in instances {
            if let Ok(Some(node)) = self.store.get_node(inst.node_id).await {
                if let Some(ref ep) = node.agent_endpoint {
                    let cmd = NodeCommand {
                        instance_id: inst.id.to_string(),
                        driver: inst.driver,
                        action: NodeAction::Stop,
                        resources: ResourceSpec {
                            vcpu: 1,
                            memory_mb: 128,
                            disk_gb: 1,
                        },
                        image_slug: None,
                    };
                    let _ = self.provider.dispatch(ep, cmd).await;
                }
            }
            inst.status = WorkloadStatus::Stopped;
            inst.updated_at = Utc::now();
            self.store.update_instance(&inst).await?;
            let _ = self.net.release(inst.id).await;
        }
        self.store
            .update_workload_status(workload_id, WorkloadStatus::Stopped)
            .await?;
        Ok(())
    }

    pub async fn register_shared_nat(
        &self,
        region: RegionCode,
        ip_or_cidr: &str,
        node_id: Option<NodeId>,
    ) -> Result<tt_core::ids::SubnetId> {
        let nid = node_id.map(|id| *id.as_uuid());
        self.net
            .register_shared_ip(region, ip_or_cidr, nid)
            .await
            .map_err(|e| ComputeError::Other(e.into()))
    }

    pub async fn list_instances(&self, workload_id: WorkloadId) -> Result<Vec<Instance>> {
        self.store.list_instances_for_workload(workload_id).await
    }

    pub async fn get_instance(&self, id: InstanceId) -> Result<Instance> {
        self.store
            .get_instance(id)
            .await?
            .ok_or(ComputeError::NotFound)
    }
}

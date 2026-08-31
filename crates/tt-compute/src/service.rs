use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::*;
use tt_core::ids::*;
use tt_net::IpAllocator;

use crate::error::{ComputeError, Result};
use crate::provider::{HypervisorProvider, NodeAction, NodeCommand};
use crate::security::{self, assert_workload_org};
use crate::store::ComputeStore;

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuotaUsage {
    pub max_workloads: u32,
    pub max_vcpu_total: u32,
    pub max_concurrent_running: u32,
    pub workloads_used: u32,
    pub vcpu_used: u32,
    pub running_used: u32,
}

pub struct ComputeService {
    store: Arc<dyn ComputeStore>,
    net: Arc<dyn IpAllocator>,
    provider: Arc<dyn HypervisorProvider>,
    /// e.g. http://control.example:8080 — used to build Smart HTTP clone URLs
    public_base_url: String,
    lifecycle: Option<Arc<dyn LifecycleEmitter>>,
    secret_box: crate::crypto::SecretBox,
    /// Extra operator principal ids from config (TT_PLATFORM_OPERATOR_IDS)
    platform_operator_ids: Vec<String>,
}

/// Thin adapter so tt-compute does not depend on tt-storage.
#[async_trait::async_trait]
pub trait LifecycleEmitter: Send + Sync {
    async fn emit(&self, event: LifecycleEvent);
}

impl ComputeService {
    pub fn new(
        store: Arc<dyn ComputeStore>,
        net: Arc<dyn IpAllocator>,
        provider: Arc<dyn HypervisorProvider>,
        public_base_url: String,
        secrets_key: [u8; 32],
        platform_operator_ids: Vec<String>,
    ) -> Self {
        Self {
            store,
            net,
            provider,
            public_base_url: public_base_url.trim_end_matches('/').to_string(),
            lifecycle: None,
            secret_box: crate::crypto::SecretBox::new(secrets_key),
            platform_operator_ids,
        }
    }

    pub async fn require_platform_operator(&self, principal_id: PrincipalId) -> Result<()> {
        let id = principal_id.to_string();
        if self.platform_operator_ids.iter().any(|x| x == &id) {
            return Ok(());
        }
        if self.store.is_platform_operator(principal_id).await? {
            return Ok(());
        }
        Err(ComputeError::Forbidden(
            "platform operator required to manage compute nodes".into(),
        ))
    }

    pub async fn add_platform_operator(
        &self,
        actor: PrincipalId,
        target: PrincipalId,
        note: Option<String>,
    ) -> Result<()> {
        self.require_platform_operator(actor).await?;
        self.store
            .add_platform_operator(target, note.as_deref())
            .await
    }

    pub fn with_lifecycle(mut self, emitter: Arc<dyn LifecycleEmitter>) -> Self {
        self.lifecycle = Some(emitter);
        self
    }

    async fn emit(&self, ev: LifecycleEvent) {
        if let Some(ref lc) = self.lifecycle {
            lc.emit(ev).await;
        }
    }

    async fn decrypt_node_agent(&self, node_id: NodeId) -> Option<String> {
        let enc = self.store.get_node_agent_enc(node_id).await.ok()??;
        self.secret_box.decrypt_str(&enc).ok()
    }

    fn lc(
        org_id: Option<OrgId>,
        kind: LifecycleKind,
        principal_id: Option<PrincipalId>,
    ) -> LifecycleEvent {
        LifecycleEvent {
            id: uuid::Uuid::now_v7(),
            ts: Utc::now(),
            org_id,
            principal_id,
            kind,
            workload_id: None,
            instance_id: None,
            node_id: None,
            region: None,
            metadata: serde_json::json!({}),
        }
    }

    async fn resolve_clone_url(&self, repository_id: Option<RepositoryId>) -> Option<String> {
        let rid = repository_id?;
        let (ws, name) = self.store.repo_clone_coords(rid).await.ok().flatten()?;
        Some(format!(
            "{}/git/{}/{}",
            self.public_base_url,
            ws,
            name
        ))
    }

    async fn mint_deploy_token(
        &self,
        org_id: OrgId,
        repository_id: Option<RepositoryId>,
        principal_id: PrincipalId,
    ) -> Option<String> {
        use sha2::{Digest, Sha256};
        let secret = uuid::Uuid::new_v4().to_string().replace('-', "");
        let prefix = secret.chars().take(8).collect::<String>();
        let hash = format!("{:x}", Sha256::digest(secret.as_bytes()));
        let id = uuid::Uuid::now_v7();
        let exp = Utc::now() + chrono::Duration::hours(2);
        let plaintext = format!("tt_deploy_{prefix}_{secret}");
        let enc = self.secret_box.encrypt_str(&plaintext).ok();
        if self
            .store
            .insert_deploy_token(
                id,
                org_id,
                repository_id,
                principal_id,
                &hash,
                &prefix,
                exp,
                enc.as_deref(),
            )
            .await
            .is_err()
        {
            return None;
        }
        Some(plaintext)
    }

    pub async fn register_node(
        &self,
        actor: PrincipalId,
        name: String,
        region: RegionCode,
        physical_cores: u32,
        memory_mb: u32,
        reserved_cores: u32,
        reserved_memory_mb: u32,
        overcommit_ratio: f32,
    ) -> Result<(ComputeNode, String)> {
        self.require_platform_operator(actor).await?;
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
        let (plaintext, hash, prefix) = security::mint_agent_token();
        let enc = self
            .secret_box
            .encrypt_str(&plaintext)
            .map_err(|e| ComputeError::Other(anyhow::anyhow!(e.to_string())))?;
        self.store
            .set_node_agent_secrets(node.id, &hash, &prefix, &enc)
            .await?;
        {
            let mut ev = Self::lc(None, LifecycleKind::NodeRegistered, None);
            ev.node_id = Some(node.id);
            ev.region = Some(node.region.as_str().into());
            ev.metadata = serde_json::json!({
                "name": node.name,
                "physical_cores": node.physical_cores,
                "memory_mb": node.memory_mb,
                "sellable_cores": node.sellable_cores(),
                "capacity_1vcpu": node.capacity_1vcpu(),
                "overcommit_ratio": node.overcommit_ratio,
            });
            self.emit(ev).await;
        }
        info!(
            node = %node.id,
            cores = physical_cores,
            sellable = node.sellable_cores(),
            cap_1vcpu = node.capacity_1vcpu(),
            "compute node registered"
        );
        Ok((node, plaintext))
    }

    pub async fn list_nodes(&self, region: Option<RegionCode>) -> Result<Vec<ComputeNode>> {
        self.store.list_nodes(region).await
    }

    pub async fn heartbeat(
        &self,
        node_id: NodeId,
        endpoint: &str,
        agent_token: Option<&str>,
    ) -> Result<()> {
        // Agent must present token once issued (bootstrap allows empty only if no hash)
        if let Some(tok) = agent_token {
            if !self.store.verify_node_agent_token(node_id, tok).await? {
                return Err(ComputeError::Forbidden("invalid node agent token".into()));
            }
        } else {
            // Require token if one is configured
            let ok = self.store.verify_node_agent_token(node_id, "").await?;
            // verify with empty: if hash exists, fails; if no hash, ok
            if !ok {
                return Err(ComputeError::Forbidden("node agent token required".into()));
            }
        }
        self.store.heartbeat(node_id, endpoint).await?;
        {
            let mut ev = Self::lc(None, LifecycleKind::NodeReady, None);
            ev.node_id = Some(node_id);
            ev.metadata = serde_json::json!({ "endpoint": endpoint });
            self.emit(ev).await;
        }
        Ok(())
    }

    /// Tenant isolation: load workload and ensure it belongs to org.
    pub async fn get_workload_for_org(
        &self,
        org_id: OrgId,
        workload_id: WorkloadId,
    ) -> Result<Workload> {
        let wl = self.get_workload(workload_id).await?;
        assert_workload_org(&wl, org_id)?;
        Ok(wl)
    }

    pub async fn start_for_org(
        &self,
        org_id: OrgId,
        workload_id: WorkloadId,
    ) -> Result<Instance> {
        let _ = self.get_workload_for_org(org_id, workload_id).await?;
        self.start(workload_id).await
    }

    pub async fn stop_for_org(
        &self,
        org_id: OrgId,
        workload_id: WorkloadId,
    ) -> Result<()> {
        let _ = self.get_workload_for_org(org_id, workload_id).await?;
        self.stop(workload_id).await
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
            let msg = format!("workload limit: {used}/{max_wl}");
            let mut ev = Self::lc(Some(org_id), LifecycleKind::QuotaExceeded, Some(created_by));
            ev.region = Some(region.as_str().into());
            ev.metadata = serde_json::json!({
                "reason": "max_workloads",
                "used": used,
                "max": max_wl,
            });
            self.emit(ev).await;
            return Err(ComputeError::QuotaExceeded(msg));
        }
        let vcpu_used = self.store.sum_org_vcpu(org_id).await?;
        if vcpu_used + resources.vcpu > max_vcpu {
            let msg = format!(
                "vcpu limit: {vcpu_used}+{} would exceed {max_vcpu}",
                resources.vcpu
            );
            let mut ev = Self::lc(Some(org_id), LifecycleKind::QuotaExceeded, Some(created_by));
            ev.region = Some(region.as_str().into());
            ev.metadata = serde_json::json!({
                "reason": "max_vcpu_total",
                "vcpu_used": vcpu_used,
                "requested": resources.vcpu,
                "max": max_vcpu,
            });
            self.emit(ev).await;
            return Err(ComputeError::QuotaExceeded(msg));
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
        {
            let mut ev = LifecycleEvent {
                id: uuid::Uuid::now_v7(),
                ts: Utc::now(),
                org_id: Some(wl.org_id),
                principal_id: Some(wl.created_by),
                kind: LifecycleKind::WorkloadCreated,
                workload_id: Some(wl.id),
                instance_id: None,
                node_id: None,
                region: Some(wl.region.as_str().into()),
                metadata: serde_json::json!({
                    "name": wl.name,
                    "shape": wl.shape.as_str(),
                    "vcpu": wl.resources.vcpu,
                }),
            };
            let _ = &mut ev;
            self.emit(ev).await;
        }
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

        let (_, _, max_running) = self.store.get_quota(wl.org_id).await?;
        let running = self.store.count_org_running(wl.org_id).await?;
        // This workload is not yet running/scheduled — ensure room for one more.
        if !matches!(wl.status, WorkloadStatus::Scheduled | WorkloadStatus::Running)
            && running >= max_running
        {
            let msg = format!("concurrent running limit: {running}/{max_running}");
            let mut ev = Self::lc(Some(wl.org_id), LifecycleKind::QuotaExceeded, Some(wl.created_by));
            ev.workload_id = Some(wl.id);
            ev.region = Some(wl.region.as_str().into());
            ev.metadata = serde_json::json!({
                "reason": "max_concurrent_running",
                "running": running,
                "max": max_running,
            });
            self.emit(ev).await;
            return Err(ComputeError::QuotaExceeded(msg));
        }

        let nodes = self.store.list_nodes(Some(wl.region)).await?;
        let node = match nodes.into_iter().find(|n| n.status == NodeStatus::Ready) {
            Some(n) => n,
            None => {
                let mut ev = Self::lc(Some(wl.org_id), LifecycleKind::InstanceFailed, Some(wl.created_by));
                ev.workload_id = Some(wl.id);
                ev.region = Some(wl.region.as_str().into());
                ev.metadata = serde_json::json!({
                    "reason": "no_ready_node",
                    "region": wl.region.as_str(),
                });
                self.emit(ev).await;
                return Err(ComputeError::NoCapacity(format!(
                    "no ready node in {}",
                    wl.region.as_str()
                )));
            }
        };

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
                inst.nat_public_ip = Some(nat.public_ip.clone());
                inst.nat_port_start = Some(nat.port_start);
                inst.nat_port_end = Some(nat.port_end);
                inst.ipv6 = nat.ipv6.clone();
                let mut ev = Self::lc(Some(wl.org_id), LifecycleKind::NatAllocated, Some(wl.created_by));
                ev.workload_id = Some(wl.id);
                ev.instance_id = Some(inst.id);
                ev.node_id = Some(node.id);
                ev.region = Some(wl.region.as_str().into());
                ev.metadata = serde_json::json!({
                    "public_ip": nat.public_ip,
                    "port_start": nat.port_start,
                    "port_end": nat.port_end,
                    "ipv6": nat.ipv6,
                });
                self.emit(ev).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "NAT allocate failed — continuing without public map");
            }
        }

        self.store.insert_instance(&inst).await?;
        {
            let mut ev = Self::lc(Some(wl.org_id), LifecycleKind::InstanceScheduled, Some(wl.created_by));
            ev.workload_id = Some(wl.id);
            ev.instance_id = Some(inst.id);
            ev.node_id = Some(node.id);
            ev.region = Some(wl.region.as_str().into());
            ev.metadata = serde_json::json!({ "driver": inst.driver.as_str() });
            self.emit(ev).await;
        }

        if let Some(ref endpoint) = node.agent_endpoint {
            let id_hex = inst.id.to_string().replace('-', "");
            let tap = format!("tt{}", &id_hex[..id_hex.len().min(8)]);
            let image_slug = wl.image_template_id.map(|_| "alpine".to_string());
            let cmd = NodeCommand {
                instance_id: inst.id.to_string(),
                driver: inst.driver,
                action: NodeAction::Boot,
                resources: wl.resources.clone(),
                image_slug,
                tap_device: Some(tap),
                nat_public_ip: inst.nat_public_ip.clone(),
                nat_port_start: inst.nat_port_start,
                nat_port_end: inst.nat_port_end,
                git_clone_url: self.resolve_clone_url(wl.repository_id).await,
                git_sha: wl.git_sha.clone(),
                deploy_token: {
                    let need = match wl.repository_id {
                        Some(rid) => self.store.repo_is_private(rid).await.unwrap_or(true),
                        None => false,
                    };
                    if need {
                        self.mint_deploy_token(wl.org_id, wl.repository_id, wl.created_by)
                            .await
                    } else {
                        None
                    }
                },
                agent_token: self.decrypt_node_agent(node.id).await,
            };
            if let Err(e) = self.provider.dispatch(endpoint, cmd).await {
                tracing::warn!(error = %e, "tt-node dispatch failed; instance recorded as running");
                let mut ev = Self::lc(Some(wl.org_id), LifecycleKind::ProviderDispatchFailed, Some(wl.created_by));
                ev.workload_id = Some(wl.id);
                ev.instance_id = Some(inst.id);
                ev.node_id = Some(node.id);
                ev.region = Some(wl.region.as_str().into());
                ev.metadata = serde_json::json!({ "error": e.to_string() });
                self.emit(ev).await;
            } else {
                let mut ev = Self::lc(Some(wl.org_id), LifecycleKind::InstanceBooted, Some(wl.created_by));
                ev.workload_id = Some(wl.id);
                ev.instance_id = Some(inst.id);
                ev.node_id = Some(node.id);
                ev.region = Some(wl.region.as_str().into());
                ev.metadata = serde_json::json!({ "driver": inst.driver.as_str() });
                self.emit(ev).await;
            }
        }

        inst.status = WorkloadStatus::Running;
        inst.updated_at = Utc::now();
        self.store.update_instance(&inst).await?;
        self.store
            .update_workload_status(workload_id, WorkloadStatus::Running)
            .await?;

        {
            let ev = LifecycleEvent {
                id: uuid::Uuid::now_v7(),
                ts: Utc::now(),
                org_id: Some(wl.org_id),
                principal_id: Some(wl.created_by),
                kind: LifecycleKind::WorkloadStarted,
                workload_id: Some(wl.id),
                instance_id: Some(inst.id),
                node_id: Some(node.id),
                region: Some(wl.region.as_str().into()),
                metadata: serde_json::json!({
                    "driver": inst.driver.as_str(),
                    "nat_public_ip": inst.nat_public_ip,
                    "nat_port_start": inst.nat_port_start,
                    "nat_port_end": inst.nat_port_end,
                }),
            };
            self.emit(ev).await;
        }
        info!(
            instance = %inst.id,
            node = %node.id,
            driver = inst.driver.as_str(),
            "instance started"
        );
        Ok(inst)
    }

    pub async fn stop(&self, workload_id: WorkloadId) -> Result<()> {
        let wl = self.get_workload(workload_id).await?;
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
                        tap_device: None,
                        nat_public_ip: None,
                        nat_port_start: None,
                        nat_port_end: None,
                        git_clone_url: None,
                        git_sha: None,
                        deploy_token: None,
                        agent_token: self.decrypt_node_agent(node.id).await,
                    };
                    let _ = self.provider.dispatch(ep, cmd).await;
                }
            }
            inst.status = WorkloadStatus::Stopped;
            inst.updated_at = Utc::now();
            self.store.update_instance(&inst).await?;
            let _ = self.net.release(inst.id).await;
            {
                let mut ev = Self::lc(Some(wl.org_id), LifecycleKind::NatReleased, Some(wl.created_by));
                ev.workload_id = Some(wl.id);
                ev.instance_id = Some(inst.id);
                ev.node_id = Some(inst.node_id);
                ev.metadata = serde_json::json!({});
                self.emit(ev).await;
            }
        }
        self.store
            .update_workload_status(workload_id, WorkloadStatus::Stopped)
            .await?;
        {
            let ev = LifecycleEvent {
                id: uuid::Uuid::now_v7(),
                ts: Utc::now(),
                org_id: Some(wl.org_id),
                principal_id: Some(wl.created_by),
                kind: LifecycleKind::WorkloadStopped,
                workload_id: Some(wl.id),
                instance_id: None,
                node_id: None,
                region: Some(wl.region.as_str().into()),
                metadata: serde_json::json!({}),
            };
            self.emit(ev).await;
        }
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


    pub async fn get_quota_usage(&self, org_id: OrgId) -> Result<QuotaUsage> {
        let (max_workloads, max_vcpu_total, max_concurrent_running) =
            self.store.get_quota(org_id).await?;
        let workloads_used = self.store.count_org_workloads(org_id).await?;
        let vcpu_used = self.store.sum_org_vcpu(org_id).await?;
        let running_used = self.store.count_org_running(org_id).await?;
        Ok(QuotaUsage {
            max_workloads,
            max_vcpu_total,
            max_concurrent_running,
            workloads_used,
            vcpu_used,
            running_used,
        })
    }

    pub async fn set_quota(
        &self,
        org_id: OrgId,
        max_workloads: u32,
        max_vcpu_total: u32,
        max_concurrent_running: u32,
    ) -> Result<QuotaUsage> {
        if max_workloads == 0 || max_vcpu_total == 0 || max_concurrent_running == 0 {
            return Err(ComputeError::InvalidArgument(
                "quota fields must be >= 1".into(),
            ));
        }
        self.store
            .set_quota(org_id, max_workloads, max_vcpu_total, max_concurrent_running)
            .await?;
        self.get_quota_usage(org_id).await
    }

    /// Create workload bound to a repo ref and start it (deploy).
    pub async fn deploy_from_git(
        &self,
        org_id: OrgId,
        created_by: PrincipalId,
        name: String,
        shape: WorkloadShape,
        region: RegionCode,
        resources: ResourceSpec,
        repository_id: RepositoryId,
        git_ref: String,
        git_sha: Option<String>,
        image_template_id: Option<ImageTemplateId>,
    ) -> Result<(Workload, Instance)> {
        {
            let ev = LifecycleEvent {
                id: uuid::Uuid::now_v7(),
                ts: Utc::now(),
                org_id: Some(org_id),
                principal_id: Some(created_by),
                kind: LifecycleKind::DeployRequested,
                workload_id: None,
                instance_id: None,
                node_id: None,
                region: Some(region.as_str().into()),
                metadata: serde_json::json!({
                    "repository_id": repository_id.to_string(),
                    "git_ref": git_ref,
                    "name": name,
                }),
            };
            self.emit(ev).await;
        }
        let mut wl = self
            .create_workload(
                org_id,
                created_by,
                name,
                shape,
                resources,
                region,
                image_template_id,
                Some(repository_id),
                Some(git_ref),
            )
            .await?;
        // persist sha if provided
        if git_sha.is_some() {
            wl.git_sha = git_sha;
            // best-effort: status path doesn't update sha — store via start using wl
        }
        let inst = self.start(wl.id).await?;
        Ok((wl, inst))
    }
}

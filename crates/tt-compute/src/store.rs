use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tt_core::domain::*;
use tt_core::ids::*;
use uuid::Uuid;

use crate::error::{ComputeError, Result};

#[async_trait]
pub trait ComputeStore: Send + Sync {
    async fn insert_node(&self, n: &ComputeNode) -> Result<()>;
    async fn get_node(&self, id: NodeId) -> Result<Option<ComputeNode>>;
    async fn list_nodes(&self, region: Option<RegionCode>) -> Result<Vec<ComputeNode>>;
    async fn heartbeat(&self, id: NodeId, endpoint: &str) -> Result<()>;
    async fn set_node_status(&self, id: NodeId, status: NodeStatus) -> Result<()>;
    async fn set_node_agent_token(&self, id: NodeId, token_hash: &str, prefix: &str) -> Result<()>;
    async fn set_node_agent_secrets(
        &self,
        id: NodeId,
        token_hash: &str,
        prefix: &str,
        token_enc: &[u8],
    ) -> Result<()>;
    async fn get_node_agent_enc(&self, id: NodeId) -> Result<Option<Vec<u8>>>;
    async fn verify_node_agent_token(&self, id: NodeId, token: &str) -> Result<bool>;
    async fn is_platform_operator(&self, principal_id: PrincipalId) -> Result<bool>;
    async fn add_platform_operator(&self, principal_id: PrincipalId, note: Option<&str>) -> Result<()>;
    async fn list_stuck_scheduled(
        &self,
        older_than: chrono::TimeDelta,
    ) -> Result<Vec<Workload>>;

    async fn insert_workload(&self, w: &Workload) -> Result<()>;
    async fn get_workload(&self, id: WorkloadId) -> Result<Option<Workload>>;
    async fn list_workloads(&self, org_id: OrgId) -> Result<Vec<Workload>>;
    async fn update_workload_status(&self, id: WorkloadId, status: WorkloadStatus) -> Result<()>;

    async fn insert_instance(&self, i: &Instance) -> Result<()>;
    async fn get_instance(&self, id: InstanceId) -> Result<Option<Instance>>;
    async fn list_instances_for_workload(&self, workload_id: WorkloadId) -> Result<Vec<Instance>>;
    async fn update_instance(&self, i: &Instance) -> Result<()>;

    async fn count_org_workloads(&self, org_id: OrgId) -> Result<u32>;
    /// Workloads currently running or scheduled (counts toward concurrent cap).
    async fn count_org_running(&self, org_id: OrgId) -> Result<u32>;
    /// Sum of vcpu across non-destroyed workloads.
    async fn sum_org_vcpu(&self, org_id: OrgId) -> Result<u32>;
    async fn get_quota(&self, org_id: OrgId) -> Result<(u32, u32, u32)>; // max_wl, max_vcpu, max_running
    async fn set_quota(
        &self,
        org_id: OrgId,
        max_workloads: u32,
        max_vcpu_total: u32,
        max_concurrent_running: u32,
    ) -> Result<()>;
    /// Ensure a quota row exists (defaults).
    async fn ensure_quota(&self, org_id: OrgId) -> Result<()>;
    /// workspace_id + repo name for Smart HTTP clone URL
    async fn repo_clone_coords(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Option<(WorkspaceId, String)>>;
    async fn repo_is_private(&self, repository_id: RepositoryId) -> Result<bool>;
    async fn insert_deploy_token(
        &self,
        id: uuid::Uuid,
        org_id: OrgId,
        repository_id: Option<RepositoryId>,
        principal_id: PrincipalId,
        token_hash: &str,
        token_prefix: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
        token_enc: Option<&[u8]>,
    ) -> Result<()>;
}

pub struct PostgresComputeStore {
    pool: PgPool,
}

impl PostgresComputeStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct NodeRow {
    id: Uuid,
    name: String,
    region: String,
    status: String,
    physical_cores: i32,
    memory_mb: i32,
    reserved_cores: i32,
    reserved_memory_mb: i32,
    overcommit_ratio: f32,
    agent_endpoint: Option<String>,
    last_heartbeat: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<NodeRow> for ComputeNode {
    fn from(r: NodeRow) -> Self {
        ComputeNode {
            id: NodeId::from_uuid(r.id),
            name: r.name,
            region: RegionCode::parse(&r.region).unwrap_or(RegionCode::EuCentral),
            status: NodeStatus::parse(&r.status).unwrap_or(NodeStatus::Offline),
            physical_cores: r.physical_cores.max(0) as u32,
            memory_mb: r.memory_mb.max(0) as u32,
            reserved_cores: r.reserved_cores.max(0) as u32,
            reserved_memory_mb: r.reserved_memory_mb.max(0) as u32,
            overcommit_ratio: r.overcommit_ratio,
            agent_endpoint: r.agent_endpoint,
            last_heartbeat: r.last_heartbeat,
            created_at: r.created_at,
        }
    }
}

#[derive(FromRow)]
struct WlRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    shape: String,
    status: String,
    vcpu: i32,
    memory_mb: i32,
    disk_gb: i32,
    image_template_id: Option<Uuid>,
    repository_id: Option<Uuid>,
    git_ref: Option<String>,
    git_sha: Option<String>,
    region: String,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<WlRow> for Workload {
    fn from(r: WlRow) -> Self {
        Workload {
            id: WorkloadId::from_uuid(r.id),
            org_id: OrgId::from_uuid(r.org_id),
            name: r.name,
            shape: WorkloadShape::parse(&r.shape).unwrap_or(WorkloadShape::Vm),
            status: WorkloadStatus::parse(&r.status).unwrap_or(WorkloadStatus::Pending),
            resources: ResourceSpec {
                vcpu: r.vcpu.max(1) as u32,
                memory_mb: r.memory_mb.max(128) as u32,
                disk_gb: r.disk_gb.max(1) as u32,
            },
            image_template_id: r.image_template_id.map(ImageTemplateId::from_uuid),
            repository_id: r.repository_id.map(RepositoryId::from_uuid),
            git_ref: r.git_ref,
            git_sha: r.git_sha,
            region: RegionCode::parse(&r.region).unwrap_or(RegionCode::EuCentral),
            created_by: PrincipalId::from_uuid(r.created_by),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(FromRow)]
struct InstRow {
    id: Uuid,
    workload_id: Uuid,
    node_id: Uuid,
    driver: String,
    status: String,
    private_ip: Option<String>,
    ipv6: Option<String>,
    nat_public_ip: Option<String>,
    nat_port_start: Option<i32>,
    nat_port_end: Option<i32>,
    dedicated_ipv4: Option<String>,
    hypervisor_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<InstRow> for Instance {
    fn from(r: InstRow) -> Self {
        let driver = match r.driver.as_str() {
            "firecracker" => HypervisorDriver::Firecracker,
            "kata" => HypervisorDriver::Kata,
            _ => HypervisorDriver::CloudHypervisor,
        };
        Instance {
            id: InstanceId::from_uuid(r.id),
            workload_id: WorkloadId::from_uuid(r.workload_id),
            node_id: NodeId::from_uuid(r.node_id),
            driver,
            status: WorkloadStatus::parse(&r.status).unwrap_or(WorkloadStatus::Pending),
            private_ip: r.private_ip,
            ipv6: r.ipv6,
            nat_public_ip: r.nat_public_ip,
            nat_port_start: r.nat_port_start.map(|p| p as u16),
            nat_port_end: r.nat_port_end.map(|p| p as u16),
            dedicated_ipv4: r.dedicated_ipv4,
            hypervisor_id: r.hypervisor_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[async_trait]
impl ComputeStore for PostgresComputeStore {
    async fn insert_node(&self, n: &ComputeNode) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO compute_nodes (
                id, name, region, status, physical_cores, memory_mb,
                reserved_cores, reserved_memory_mb, overcommit_ratio,
                agent_endpoint, last_heartbeat, created_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
            "#,
        )
        .bind(n.id.as_uuid())
        .bind(&n.name)
        .bind(n.region.as_str())
        .bind(n.status.as_str())
        .bind(n.physical_cores as i32)
        .bind(n.memory_mb as i32)
        .bind(n.reserved_cores as i32)
        .bind(n.reserved_memory_mb as i32)
        .bind(n.overcommit_ratio)
        .bind(&n.agent_endpoint)
        .bind(n.last_heartbeat)
        .bind(n.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn get_node(&self, id: NodeId) -> Result<Option<ComputeNode>> {
        let row = sqlx::query_as::<_, NodeRow>(
            "SELECT * FROM compute_nodes WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_nodes(&self, region: Option<RegionCode>) -> Result<Vec<ComputeNode>> {
        let rows = if let Some(r) = region {
            sqlx::query_as::<_, NodeRow>(
                "SELECT * FROM compute_nodes WHERE region = $1 ORDER BY name",
            )
            .bind(r.as_str())
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, NodeRow>("SELECT * FROM compute_nodes ORDER BY region, name")
                .fetch_all(&self.pool)
                .await
        }
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn heartbeat(&self, id: NodeId, endpoint: &str) -> Result<()> {
        sqlx::query(
            "UPDATE compute_nodes SET last_heartbeat = NOW(), agent_endpoint = $2, status = 'ready' WHERE id = $1",
        )
        .bind(id.as_uuid())
        .bind(endpoint)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn set_node_status(&self, id: NodeId, status: NodeStatus) -> Result<()> {
        sqlx::query("UPDATE compute_nodes SET status = $2 WHERE id = $1")
            .bind(id.as_uuid())
            .bind(status.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn set_node_agent_token(&self, id: NodeId, token_hash: &str, prefix: &str) -> Result<()> {
        sqlx::query(
            "UPDATE compute_nodes SET agent_token_hash = $2, agent_token_prefix = $3 WHERE id = $1",
        )
        .bind(id.as_uuid())
        .bind(token_hash)
        .bind(prefix)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn set_node_agent_secrets(
        &self,
        id: NodeId,
        token_hash: &str,
        prefix: &str,
        token_enc: &[u8],
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE compute_nodes
            SET agent_token_hash = $2, agent_token_prefix = $3, agent_token_enc = $4
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .bind(token_hash)
        .bind(prefix)
        .bind(token_enc)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn get_node_agent_enc(&self, id: NodeId) -> Result<Option<Vec<u8>>> {
        let row: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
            "SELECT agent_token_enc FROM compute_nodes WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(row.and_then(|r| r.0))
    }

    async fn is_platform_operator(&self, principal_id: PrincipalId) -> Result<bool> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM platform_operators WHERE principal_id = $1",
        )
        .bind(principal_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(n > 0)
    }

    async fn add_platform_operator(&self, principal_id: PrincipalId, note: Option<&str>) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO platform_operators (principal_id, note, created_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (principal_id) DO NOTHING
            "#,
        )
        .bind(principal_id.as_uuid())
        .bind(note)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn verify_node_agent_token(&self, id: NodeId, token: &str) -> Result<bool> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT agent_token_hash FROM compute_nodes WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        let Some((Some(hash),)) = row else {
            // No token set yet — allow first heartbeat (bootstrap), then lock
            return Ok(true);
        };
        Ok(crate::security::verify_agent_token(token, &hash))
    }

    async fn list_stuck_scheduled(
        &self,
        older_than: chrono::TimeDelta,
    ) -> Result<Vec<Workload>> {
        let cutoff = chrono::Utc::now() - older_than;
        let rows = sqlx::query_as::<_, WlRow>(
            r#"
            SELECT * FROM workloads
            WHERE status = 'scheduled' AND updated_at < $1
            ORDER BY updated_at ASC
            LIMIT 100
            "#,
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn insert_workload(&self, w: &Workload) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO workloads (
                id, org_id, name, shape, status, vcpu, memory_mb, disk_gb,
                image_template_id, repository_id, git_ref, git_sha, region,
                created_by, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
            "#,
        )
        .bind(w.id.as_uuid())
        .bind(w.org_id.as_uuid())
        .bind(&w.name)
        .bind(w.shape.as_str())
        .bind(w.status.as_str())
        .bind(w.resources.vcpu as i32)
        .bind(w.resources.memory_mb as i32)
        .bind(w.resources.disk_gb as i32)
        .bind(w.image_template_id.as_ref().map(|id| *id.as_uuid()))
        .bind(w.repository_id.as_ref().map(|id| *id.as_uuid()))
        .bind(&w.git_ref)
        .bind(&w.git_sha)
        .bind(w.region.as_str())
        .bind(w.created_by.as_uuid())
        .bind(w.created_at)
        .bind(w.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn get_workload(&self, id: WorkloadId) -> Result<Option<Workload>> {
        let row = sqlx::query_as::<_, WlRow>("SELECT * FROM workloads WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_workloads(&self, org_id: OrgId) -> Result<Vec<Workload>> {
        let rows = sqlx::query_as::<_, WlRow>(
            "SELECT * FROM workloads WHERE org_id = $1 ORDER BY created_at DESC",
        )
        .bind(org_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_workload_status(&self, id: WorkloadId, status: WorkloadStatus) -> Result<()> {
        sqlx::query("UPDATE workloads SET status = $2, updated_at = NOW() WHERE id = $1")
            .bind(id.as_uuid())
            .bind(status.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn insert_instance(&self, i: &Instance) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO instances (
                id, workload_id, node_id, driver, status, private_ip, ipv6,
                nat_public_ip, nat_port_start, nat_port_end, dedicated_ipv4,
                hypervisor_id, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
            "#,
        )
        .bind(i.id.as_uuid())
        .bind(i.workload_id.as_uuid())
        .bind(i.node_id.as_uuid())
        .bind(i.driver.as_str())
        .bind(i.status.as_str())
        .bind(&i.private_ip)
        .bind(&i.ipv6)
        .bind(&i.nat_public_ip)
        .bind(i.nat_port_start.map(|p| p as i32))
        .bind(i.nat_port_end.map(|p| p as i32))
        .bind(&i.dedicated_ipv4)
        .bind(&i.hypervisor_id)
        .bind(i.created_at)
        .bind(i.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn get_instance(&self, id: InstanceId) -> Result<Option<Instance>> {
        let row = sqlx::query_as::<_, InstRow>("SELECT * FROM instances WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_instances_for_workload(&self, workload_id: WorkloadId) -> Result<Vec<Instance>> {
        let rows = sqlx::query_as::<_, InstRow>(
            "SELECT * FROM instances WHERE workload_id = $1 ORDER BY created_at DESC",
        )
        .bind(workload_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_instance(&self, i: &Instance) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE instances SET
                status = $2, private_ip = $3, ipv6 = $4,
                nat_public_ip = $5, nat_port_start = $6, nat_port_end = $7,
                dedicated_ipv4 = $8, hypervisor_id = $9, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(i.id.as_uuid())
        .bind(i.status.as_str())
        .bind(&i.private_ip)
        .bind(&i.ipv6)
        .bind(&i.nat_public_ip)
        .bind(i.nat_port_start.map(|p| p as i32))
        .bind(i.nat_port_end.map(|p| p as i32))
        .bind(&i.dedicated_ipv4)
        .bind(&i.hypervisor_id)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn count_org_workloads(&self, org_id: OrgId) -> Result<u32> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM workloads WHERE org_id = $1 AND status <> 'destroyed'",
        )
        .bind(org_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(n.max(0) as u32)
    }

    async fn count_org_running(&self, org_id: OrgId) -> Result<u32> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint FROM workloads
            WHERE org_id = $1 AND status IN ('running', 'scheduled')
            "#,
        )
        .bind(org_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(n.max(0) as u32)
    }

    async fn sum_org_vcpu(&self, org_id: OrgId) -> Result<u32> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(vcpu), 0)::bigint FROM workloads
            WHERE org_id = $1 AND status <> 'destroyed'
            "#,
        )
        .bind(org_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(n.max(0) as u32)
    }

    async fn get_quota(&self, org_id: OrgId) -> Result<(u32, u32, u32)> {
        self.ensure_quota(org_id).await?;
        let row: Option<(i32, i32, i32)> = sqlx::query_as(
            "SELECT max_workloads, max_vcpu_total, max_concurrent_running FROM org_compute_quotas WHERE org_id = $1",
        )
        .bind(org_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(match row {
            Some((a, b, c)) => (a.max(0) as u32, b.max(0) as u32, c.max(0) as u32),
            None => (5, 8, 5),
        })
    }

    async fn set_quota(
        &self,
        org_id: OrgId,
        max_workloads: u32,
        max_vcpu_total: u32,
        max_concurrent_running: u32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO org_compute_quotas (
                org_id, max_workloads, max_vcpu_total, max_concurrent_running, updated_at
            ) VALUES ($1, $2, $3, $4, NOW())
            ON CONFLICT (org_id) DO UPDATE SET
                max_workloads = EXCLUDED.max_workloads,
                max_vcpu_total = EXCLUDED.max_vcpu_total,
                max_concurrent_running = EXCLUDED.max_concurrent_running,
                updated_at = NOW()
            "#,
        )
        .bind(org_id.as_uuid())
        .bind(max_workloads as i32)
        .bind(max_vcpu_total as i32)
        .bind(max_concurrent_running as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn ensure_quota(&self, org_id: OrgId) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO org_compute_quotas (org_id, max_workloads, max_vcpu_total, max_concurrent_running, updated_at)
            VALUES ($1, 5, 8, 5, NOW())
            ON CONFLICT (org_id) DO NOTHING
            "#,
        )
        .bind(org_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }

    async fn repo_clone_coords(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Option<(WorkspaceId, String)>> {
        let row: Option<(uuid::Uuid, String)> = sqlx::query_as(
            "SELECT workspace_id, name FROM repositories WHERE id = $1",
        )
        .bind(repository_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(row.map(|(ws, name)| (WorkspaceId::from_uuid(ws), name)))
    }

    async fn repo_is_private(&self, repository_id: RepositoryId) -> Result<bool> {
        let row: Option<(bool,)> = sqlx::query_as(
            "SELECT is_private FROM repositories WHERE id = $1",
        )
        .bind(repository_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(row.map(|r| r.0).unwrap_or(true))
    }

    async fn insert_deploy_token(
        &self,
        id: uuid::Uuid,
        org_id: OrgId,
        repository_id: Option<RepositoryId>,
        principal_id: PrincipalId,
        token_hash: &str,
        token_prefix: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
        token_enc: Option<&[u8]>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO deploy_tokens (
                id, org_id, repository_id, principal_id, token_hash, token_prefix, expires_at, token_enc
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            "#,
        )
        .bind(id)
        .bind(org_id.as_uuid())
        .bind(repository_id.as_ref().map(|r| *r.as_uuid()))
        .bind(principal_id.as_uuid())
        .bind(token_hash)
        .bind(token_prefix)
        .bind(expires_at)
        .bind(token_enc)
        .execute(&self.pool)
        .await
        .map_err(|e| ComputeError::Other(e.into()))?;
        Ok(())
    }
}

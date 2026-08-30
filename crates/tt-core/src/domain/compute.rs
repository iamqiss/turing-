//! Compute lifecycle — workloads, instances, nodes, shapes.
//!
//! Inspired by CloudRat economics: bare metal + overcommit + NAT/IPv6,
//! but control plane is Turing Tarpit (Rust), hypervisors are FC/CH/Kata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{
    ImageTemplateId, InstanceId, NodeId, OrgId, PrincipalId, RepositoryId, SubnetId, WorkloadId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionCode {
    UsEast,
    EuCentral,
    AfSouth, // future
}

impl RegionCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsEast => "us-east",
            Self::EuCentral => "eu-central",
            Self::AfSouth => "af-south",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "us-east" => Some(Self::UsEast),
            "eu-central" => Some(Self::EuCentral),
            "af-south" => Some(Self::AfSouth),
            _ => None,
        }
    }
}

/// User-facing product shape (hides hypervisor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadShape {
    /// Firecracker microVM — serverless / agents / CI
    Serverless,
    /// Cloud Hypervisor — persistent “VPS”
    Vm,
    /// Kata — container UX, VM isolation
    Kata,
}

impl WorkloadShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Serverless => "serverless",
            Self::Vm => "vm",
            Self::Kata => "kata",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "serverless" => Some(Self::Serverless),
            "vm" => Some(Self::Vm),
            "kata" => Some(Self::Kata),
            _ => None,
        }
    }

    pub fn preferred_driver(self) -> HypervisorDriver {
        match self {
            Self::Serverless => HypervisorDriver::Firecracker,
            Self::Vm => HypervisorDriver::CloudHypervisor,
            Self::Kata => HypervisorDriver::Kata,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypervisorDriver {
    Firecracker,
    CloudHypervisor,
    Kata,
}

impl HypervisorDriver {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firecracker => "firecracker",
            Self::CloudHypervisor => "cloud_hypervisor",
            Self::Kata => "kata",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Registering,
    Ready,
    Draining,
    Offline,
}

impl NodeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registering => "registering",
            Self::Ready => "ready",
            Self::Draining => "draining",
            Self::Offline => "offline",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "registering" => Some(Self::Registering),
            "ready" => Some(Self::Ready),
            "draining" => Some(Self::Draining),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }
}

/// Bare-metal host running tt-node (OVH / Latitude / Hetzner / …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeNode {
    pub id: NodeId,
    pub name: String,
    pub region: RegionCode,
    pub status: NodeStatus,
    /// Physical cores (not threads) — CloudRat lesson: never lie about this.
    pub physical_cores: u32,
    pub memory_mb: u32,
    /// Reserved for TT / internal quarantine (Entativa-style shield).
    pub reserved_cores: u32,
    pub reserved_memory_mb: u32,
    /// CPU overcommit ratio (e.g. 4 = 4:1).
    pub overcommit_ratio: f32,
    pub agent_endpoint: Option<String>,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ComputeNode {
    pub fn sellable_cores(&self) -> u32 {
        self.physical_cores.saturating_sub(self.reserved_cores)
    }

    /// Theoretical max 1-vCPU instances at configured overcommit.
    pub fn capacity_1vcpu(&self) -> u32 {
        ((self.sellable_cores() as f32) * self.overcommit_ratio).floor() as u32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    pub vcpu: u32,
    pub memory_mb: u32,
    pub disk_gb: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadStatus {
    Pending,
    Scheduled,
    Running,
    Stopped,
    Suspended,
    Failed,
    Destroyed,
}

impl WorkloadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Scheduled => "scheduled",
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Suspended => "suspended",
            Self::Failed => "failed",
            Self::Destroyed => "destroyed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "scheduled" => Some(Self::Scheduled),
            "running" => Some(Self::Running),
            "stopped" => Some(Self::Stopped),
            "suspended" => Some(Self::Suspended),
            "failed" => Some(Self::Failed),
            "destroyed" => Some(Self::Destroyed),
            _ => None,
        }
    }
}

/// Desired state — what the user/org asked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workload {
    pub id: WorkloadId,
    pub org_id: OrgId,
    pub name: String,
    pub shape: WorkloadShape,
    pub status: WorkloadStatus,
    pub resources: ResourceSpec,
    pub image_template_id: Option<ImageTemplateId>,
    /// Optional deploy-from-git
    pub repository_id: Option<RepositoryId>,
    pub git_ref: Option<String>,
    pub git_sha: Option<String>,
    pub region: RegionCode,
    pub created_by: PrincipalId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Actual running (or stopped) machine on a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: InstanceId,
    pub workload_id: WorkloadId,
    pub node_id: NodeId,
    pub driver: HypervisorDriver,
    pub status: WorkloadStatus,
    pub private_ip: Option<String>,
    pub ipv6: Option<String>,
    /// Shared NAT: public IP + port range (CloudRat model).
    pub nat_public_ip: Option<String>,
    pub nat_port_start: Option<u16>,
    pub nat_port_end: Option<u16>,
    pub dedicated_ipv4: Option<String>,
    pub hypervisor_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpKind {
    /// Dedicated public IPv4 add-on
    DedicatedV4,
    /// Shared NAT public IPv4 (pool)
    SharedNatV4,
    /// Dedicated IPv6 /64 (default free)
    DedicatedV6,
}

impl IpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DedicatedV4 => "dedicated_v4",
            Self::SharedNatV4 => "shared_nat_v4",
            Self::DedicatedV6 => "dedicated_v6",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subnet {
    pub id: SubnetId,
    pub region: RegionCode,
    pub cidr: String,
    pub kind: IpKind,
    pub node_id: Option<NodeId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageTemplate {
    pub id: ImageTemplateId,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    /// Approximate disk footprint MB (CloudRat: keep templates small)
    pub disk_mb: u32,
    pub active: bool,
}

//! Turing Tarpit Cloud control-plane logic.

mod error;
mod store;
mod service;
mod security;
mod crypto;
mod orchestrator;
pub mod provider;

pub use error::{ComputeError, Result};
pub use service::{ComputeService, LifecycleEmitter, QuotaUsage};
pub use store::{ComputeStore, PostgresComputeStore};
pub use provider::{HypervisorProvider, HttpProvider, NodeCommand, NullProvider, ProviderError};
pub use orchestrator::{Orchestrator, OrchestratorConfig};
pub use security::{assert_workload_org, hash_agent_token, mint_agent_token};

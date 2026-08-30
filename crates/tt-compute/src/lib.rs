//! Turing Tarpit Cloud control-plane logic.

mod error;
mod store;
mod service;
pub mod provider;

pub use error::{ComputeError, Result};
pub use service::ComputeService;
pub use store::{ComputeStore, PostgresComputeStore};
pub use provider::{HypervisorProvider, HttpProvider, NodeCommand, NullProvider, ProviderError};

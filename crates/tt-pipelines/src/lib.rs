//! Turing Tasks — CI/CD from `.tt/pipelines/*.yml`, billed as `tasks.minutes`.

pub mod error;
pub mod service;
pub mod store;
pub mod yaml;

pub use error::{PipelineError, Result};
pub use service::{included_minutes, TaskService};
pub use store::{PostgresTaskStore, TaskStore};
pub use yaml::{matches_trigger, parse_pipeline_yaml, PipelineFile};

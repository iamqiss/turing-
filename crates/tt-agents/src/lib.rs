//! Turing Tarpit agents — OLMo tool harness, permissions, durable runs.

mod error;
mod inference;
mod permissions;
mod prompt;
mod store;
mod tools;
mod redact;
pub mod harness;

pub use error::{AgentError, Result};
pub use harness::AgentHarness;
pub use inference::InferenceClient;
pub use store::{AgentStore, PostgresAgentStore};
pub use tools::{
    BuiltinTools, ComputeTool, GitTool, LifecycleTool, SearchTool, SecretsTool, ToolCall,
    ToolContext, ToolExecutor, ToolOutput, ToolSpec,
};
pub use permissions::{DangerLevel, ToolPolicy};

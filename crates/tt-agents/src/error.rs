use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("not found")]
    NotFound,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("model: {0}")]
    Model(String),
    #[error("tool: {0}")]
    Tool(String),
    #[error("approval required")]
    ApprovalRequired,
    #[error("max steps exceeded")]
    MaxSteps,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AgentError>;

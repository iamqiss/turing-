use thiserror::Error;

pub type Result<T> = std::result::Result<T, ComputeError>;

#[derive(Debug, Error)]
pub enum ComputeError {
    #[error("not found")]
    NotFound,
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error("no capacity: {0}")]
    NoCapacity(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("provider: {0}")]
    Provider(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

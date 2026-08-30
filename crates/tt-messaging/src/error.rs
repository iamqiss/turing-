use thiserror::Error;

pub type Result<T> = std::result::Result<T, MessagingError>;

#[derive(Debug, Error)]
pub enum MessagingError {
    #[error("not found")]
    NotFound,
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("already exists")]
    AlreadyExists,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

//! Git hosting errors.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, GitError>;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("repository not found")]
    NotFound,

    #[error("pull request not found")]
    PrNotFound,

    #[error("repository already exists")]
    AlreadyExists,

    #[error("invalid repository name: {0}")]
    InvalidName(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("git error: {0}")]
    Git(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

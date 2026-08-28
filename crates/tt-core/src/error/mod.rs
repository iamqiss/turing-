//! Core error type.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("unauthenticated")]
    Unauthenticated,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("internal: {0}")]
    Internal(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    pub fn not_found(entity: &str) -> Self {
        Self::NotFound(entity.to_string())
    }

    pub fn already_exists(entity: &str) -> Self {
        Self::AlreadyExists(entity.to_string())
    }

    pub fn permission_denied(reason: &str) -> Self {
        Self::PermissionDenied(reason.to_string())
    }

    pub fn invalid(msg: &str) -> Self {
        Self::InvalidArgument(msg.to_string())
    }
}

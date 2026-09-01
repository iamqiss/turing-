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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        assert_eq!(MessagingError::NotFound.to_string(), "not found");
        assert!(MessagingError::Forbidden("x".into()).to_string().contains("forbidden"));
        assert!(MessagingError::InvalidArgument("bad".into()).to_string().contains("bad"));
    }
}

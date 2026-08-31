use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretsError {
    #[error("not found")]
    NotFound,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, SecretsError>;

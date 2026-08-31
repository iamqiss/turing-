use thiserror::Error;

#[derive(Debug, Error)]
pub enum KmsError {
    #[error("sealed / locked — unseal first")]
    Locked,
    #[error("unknown key id: {0}")]
    UnknownKey(String),
    #[error("auth failed")]
    Auth,
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, KmsError>;

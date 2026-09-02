use thiserror::Error;

pub type Result<T> = std::result::Result<T, BillingError>;

#[derive(Debug, Error)]
pub enum BillingError {
    #[error("not found")]
    NotFound,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

use thiserror::Error;

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("not found")]
    NotFound,
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("clickhouse: {0}")]
    ClickHouse(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("meilisearch unavailable: {0}")]
    Unavailable(String),
    #[error("meilisearch: {0}")]
    Meili(String),
    #[error("disabled")]
    Disabled,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, SearchError>;

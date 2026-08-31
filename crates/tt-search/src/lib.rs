//! Turing Tarpit search — Meilisearch indexing and federated query.

mod client;
mod documents;
mod error;
mod service;
mod reindex;

pub use client::MeiliClient;
pub use documents::{
    IssueDoc, MessageDoc, PersonDoc, PrDoc, RepoDoc, IDX_ISSUES, IDX_MESSAGES, IDX_PEOPLE, IDX_PRS,
    IDX_REPOS,
};
pub use error::{Result, SearchError};
pub use service::SearchService;
pub use reindex::ReindexReport;

//! Git hosting powered by Gitoxide + Smart HTTP + pull requests.

pub mod error;
pub mod repo;
pub mod smart_http;
pub mod hooks;
pub mod pr;
pub mod merge;
pub mod protect;
pub mod review;
pub mod object_store;

pub use error::{GitError, Result};
pub use repo::{PostgresRepoStore, PostgresSettingsStore, RepoService, RepoStore, SettingsStore};
pub use smart_http::{router as git_http_router, GitHttpState};
pub use pr::{PostgresPullRequestStore, PullRequestService, PullRequestStore};

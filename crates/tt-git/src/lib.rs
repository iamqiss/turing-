//! Git hosting powered by Gitoxide + Smart HTTP + pull requests.

pub mod error;
pub mod repo;
pub mod smart_http;
pub mod hooks;
pub mod pr;
pub mod issue;
pub mod merge;
pub mod protect;
pub mod analytics;
pub mod review;
pub mod object_store;

pub use error::{GitError, Result};
pub use repo::{PostgresRepoStore, PostgresSettingsStore, RepoService, RepoStore, SettingsStore};
pub use smart_http::{router as git_http_router, GitHttpState};
pub use analytics::PushAnalysis;
pub use pr::{PostgresPullRequestStore, PullRequestService, PullRequestStore};
pub use issue::{IssueService, IssueStore, PostgresIssueStore, PostgresLabelStore};
pub use issue::closing::parse_closing_issue_numbers;
pub use review::{PostgresReviewStore, ReviewService, ReviewStore};

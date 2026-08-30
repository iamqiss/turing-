//! Pull request reviews (approve / request changes / comment).

mod store;

pub use store::{PostgresReviewStore, ReviewStore};

use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::{PullRequestStatus, Review, ReviewState};
use tt_core::ids::{PrincipalId, PullRequestId, ReviewId};

use crate::error::{GitError, Result};
use crate::pr::PullRequestStore;

pub struct ReviewService {
    store: Arc<dyn ReviewStore>,
    prs: Arc<dyn PullRequestStore>,
}

impl ReviewService {
    pub fn new(store: Arc<dyn ReviewStore>, prs: Arc<dyn PullRequestStore>) -> Self {
        Self { store, prs }
    }

    pub async fn submit(
        &self,
        pull_request_id: PullRequestId,
        author_id: PrincipalId,
        state: ReviewState,
        body: Option<String>,
    ) -> Result<Review> {
        let pr = self
            .prs
            .get(pull_request_id)
            .await?
            .ok_or(GitError::PrNotFound)?;
        if pr.status != PullRequestStatus::Open {
            return Err(GitError::Conflict("can only review open pull requests".into()));
        }
        if pr.author_id == author_id && state == ReviewState::Approve {
            return Err(GitError::InvalidArgument(
                "cannot approve your own pull request".into(),
            ));
        }
        let review = Review {
            id: ReviewId::new(),
            pull_request_id,
            author_id,
            state,
            body: body.map(|b| b.trim().to_string()).filter(|b| !b.is_empty()),
            created_at: Utc::now(),
        };
        self.store.insert(&review).await?;
        info!(
            review_id = %review.id,
            pr = %pull_request_id,
            state = review.state.as_str(),
            "review submitted"
        );
        Ok(review)
    }

    pub async fn list(&self, pull_request_id: PullRequestId) -> Result<Vec<Review>> {
        self.store.list(pull_request_id).await
    }

    /// Count distinct authors whose *latest* review is Approve.
    pub async fn approval_count(&self, pull_request_id: PullRequestId) -> Result<u32> {
        self.store.count_approvals(pull_request_id).await
    }
}

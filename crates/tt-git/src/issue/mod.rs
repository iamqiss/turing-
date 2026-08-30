//! Issue tracking per repository.

mod store;
mod labels;
pub mod closing;

pub use labels::{LabelStore, PostgresLabelStore};

pub use store::{IssueStore, PostgresIssueStore};

use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::{Issue, IssueComment, IssueState, Label};
use tt_core::ids::LabelId;
use tt_core::ids::{IssueCommentId, IssueId, PrincipalId, RepositoryId};

use crate::error::{GitError, Result};
use crate::repo::RepoService;

pub struct IssueService {
    store: Arc<dyn IssueStore>,
    labels: Arc<dyn LabelStore>,
    repos: Arc<RepoService>,
}

impl IssueService {
    pub fn new(
        store: Arc<dyn IssueStore>,
        labels: Arc<dyn LabelStore>,
        repos: Arc<RepoService>,
    ) -> Self {
        Self { store, labels, repos }
    }

    pub async fn create(
        &self,
        repository_id: RepositoryId,
        author_id: PrincipalId,
        title: String,
        body: Option<String>,
    ) -> Result<Issue> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(GitError::InvalidArgument("title required".into()));
        }
        let _ = self.repos.get(repository_id).await?;
        let number = self.store.next_number(repository_id).await?;
        let now = Utc::now();
        let issue = Issue {
            id: IssueId::new(),
            repository_id,
            number,
            title,
            body,
            author_id,
            state: IssueState::Open,
            closed_at: None,
            created_at: now,
            updated_at: now,
        };
        self.store.insert(&issue).await?;
        info!(issue_id = %issue.id, number = issue.number, "issue opened");
        Ok(issue)
    }

    pub async fn get(&self, id: IssueId) -> Result<Issue> {
        self.store.get(id).await?.ok_or(GitError::NotFound)
    }

    pub async fn get_by_number(
        &self,
        repository_id: RepositoryId,
        number: i32,
    ) -> Result<Issue> {
        self.store
            .get_by_number(repository_id, number)
            .await?
            .ok_or(GitError::NotFound)
    }

    pub async fn list(
        &self,
        repository_id: RepositoryId,
        state: Option<IssueState>,
        limit: u32,
    ) -> Result<Vec<Issue>> {
        let _ = self.repos.get(repository_id).await?;
        self.store
            .list(repository_id, state, limit.clamp(1, 100))
            .await
    }

    pub async fn update(
        &self,
        id: IssueId,
        actor_id: PrincipalId,
        title: Option<String>,
        body: Option<Option<String>>,
    ) -> Result<Issue> {
        let mut issue = self.get(id).await?;
        if issue.author_id != actor_id {
            // allow any auth user for now; tighten with roles later
        }
        if let Some(t) = title {
            let t = t.trim().to_string();
            if t.is_empty() {
                return Err(GitError::InvalidArgument("title required".into()));
            }
            issue.title = t;
        }
        if let Some(b) = body {
            issue.body = b;
        }
        issue.updated_at = Utc::now();
        self.store.update(&issue).await?;
        Ok(issue)
    }

    pub async fn close(&self, id: IssueId, _actor_id: PrincipalId) -> Result<Issue> {
        let mut issue = self.get(id).await?;
        if issue.state != IssueState::Open {
            return Err(GitError::Conflict("issue is not open".into()));
        }
        let now = Utc::now();
        issue.state = IssueState::Closed;
        issue.closed_at = Some(now);
        issue.updated_at = now;
        self.store.update(&issue).await?;
        info!(issue_id = %issue.id, number = issue.number, "issue closed");
        Ok(issue)
    }

    pub async fn reopen(&self, id: IssueId, _actor_id: PrincipalId) -> Result<Issue> {
        let mut issue = self.get(id).await?;
        if issue.state != IssueState::Closed {
            return Err(GitError::Conflict("issue is not closed".into()));
        }
        issue.state = IssueState::Open;
        issue.closed_at = None;
        issue.updated_at = Utc::now();
        self.store.update(&issue).await?;
        info!(issue_id = %issue.id, number = issue.number, "issue reopened");
        Ok(issue)
    }

    pub async fn add_comment(
        &self,
        issue_id: IssueId,
        author_id: PrincipalId,
        body: String,
    ) -> Result<IssueComment> {
        let body = body.trim().to_string();
        if body.is_empty() {
            return Err(GitError::InvalidArgument("comment body required".into()));
        }
        let _ = self.get(issue_id).await?;
        let now = Utc::now();
        let c = IssueComment {
            id: IssueCommentId::new(),
            issue_id,
            author_id,
            body,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_comment(&c).await?;
        Ok(c)
    }

    pub async fn list_comments(&self, issue_id: IssueId, limit: u32) -> Result<Vec<IssueComment>> {
        let _ = self.get(issue_id).await?;
        self.store.list_comments(issue_id, limit.clamp(1, 200)).await
    }


    pub async fn create_label(
        &self,
        repository_id: RepositoryId,
        name: String,
        color: String,
        description: Option<String>,
    ) -> Result<Label> {
        let _ = self.repos.get(repository_id).await?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(GitError::InvalidArgument("label name required".into()));
        }
        let color = color.trim().trim_start_matches('#').to_string();
        if color.len() != 6 || !color.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(GitError::InvalidArgument(
                "color must be 6 hex digits".into(),
            ));
        }
        let label = Label {
            id: LabelId::new(),
            repository_id,
            name,
            color,
            description,
        };
        self.labels.create(&label).await?;
        Ok(label)
    }

    pub async fn list_labels(&self, repository_id: RepositoryId) -> Result<Vec<Label>> {
        let _ = self.repos.get(repository_id).await?;
        self.labels.list(repository_id).await
    }

    pub async fn set_issue_labels(
        &self,
        issue_id: IssueId,
        label_names: &[String],
    ) -> Result<Vec<Label>> {
        let issue = self.get(issue_id).await?;
        let mut ids = Vec::new();
        for name in label_names {
            let label = self
                .labels
                .get_by_name(issue.repository_id, name)
                .await?
                .ok_or_else(|| GitError::InvalidArgument(format!("unknown label: {name}")))?;
            ids.push(label.id);
        }
        self.labels.set_issue_labels(issue_id, &ids).await?;
        self.labels.labels_for_issue(issue_id).await
    }

    pub async fn labels_for_issue(&self, issue_id: IssueId) -> Result<Vec<Label>> {
        self.labels.labels_for_issue(issue_id).await
    }

    /// Close issue by number if open (used by Fixes #N).
    pub async fn close_by_number(
        &self,
        repository_id: RepositoryId,
        number: i32,
        actor_id: PrincipalId,
    ) -> Result<Option<Issue>> {
        match self.store.get_by_number(repository_id, number).await? {
            Some(issue) if issue.state == IssueState::Open => {
                Ok(Some(self.close(issue.id, actor_id).await?))
            }
            Some(_) => Ok(None), // already closed
            None => Ok(None),
        }
    }
}

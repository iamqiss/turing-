//! Pull request persistence and application service.

mod store;

pub use store::{PostgresPullRequestStore, PullRequestStore};

use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::{MergeMethod, PullRequest, PullRequestStatus, Repository};
use tt_core::ids::{PrincipalId, PullRequestId, RepositoryId};

use crate::error::{GitError, Result};
use crate::repo::RepoService;

pub struct PullRequestService {
    store: Arc<dyn PullRequestStore>,
    repos: Arc<RepoService>,
}

impl PullRequestService {
    pub fn new(store: Arc<dyn PullRequestStore>, repos: Arc<RepoService>) -> Self {
        Self { store, repos }
    }

    pub async fn create(
        &self,
        repository_id: RepositoryId,
        author_id: PrincipalId,
        title: String,
        body: Option<String>,
        source_branch: String,
        target_branch: String,
    ) -> Result<PullRequest> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(GitError::InvalidArgument("title required".into()));
        }
        if source_branch.trim().is_empty() || target_branch.trim().is_empty() {
            return Err(GitError::InvalidArgument(
                "source_branch and target_branch required".into(),
            ));
        }
        if source_branch == target_branch {
            return Err(GitError::InvalidArgument(
                "source and target branches must differ".into(),
            ));
        }

        // Ensure repository exists
        let _repo = self.repos.get(repository_id).await?;

        let number = self.store.next_number(repository_id).await?;
        let now = Utc::now();
        let pr = PullRequest {
            id: PullRequestId::new(),
            repository_id,
            number,
            title,
            body,
            author_id,
            source_branch: source_branch.trim().to_string(),
            target_branch: target_branch.trim().to_string(),
            status: PullRequestStatus::Open,
            merged_by: None,
            merged_at: None,
            closed_at: None,
            created_at: now,
            updated_at: now,
        };
        self.store.insert(&pr).await?;
        info!(
            pr_id = %pr.id,
            number = pr.number,
            repository_id = %repository_id,
            "pull request opened"
        );
        Ok(pr)
    }

    pub async fn get(&self, id: PullRequestId) -> Result<PullRequest> {
        self.store.get(id).await?.ok_or(GitError::PrNotFound)
    }

    pub async fn get_by_number(
        &self,
        repository_id: RepositoryId,
        number: i32,
    ) -> Result<PullRequest> {
        self.store
            .get_by_number(repository_id, number)
            .await?
            .ok_or(GitError::PrNotFound)
    }

    pub async fn list(
        &self,
        repository_id: RepositoryId,
        status: Option<PullRequestStatus>,
        limit: u32,
    ) -> Result<Vec<PullRequest>> {
        // Ensure repo exists
        let _ = self.repos.get(repository_id).await?;
        self.store.list(repository_id, status, limit.clamp(1, 100)).await
    }

    pub async fn update(
        &self,
        id: PullRequestId,
        actor_id: PrincipalId,
        title: Option<String>,
        body: Option<Option<String>>,
    ) -> Result<PullRequest> {
        let mut pr = self.get(id).await?;
        if pr.status != PullRequestStatus::Open {
            return Err(GitError::Conflict("only open PRs can be edited".into()));
        }
        // Author can edit; tighten with repo perms later
        if pr.author_id != actor_id {
            return Err(GitError::PermissionDenied(
                "only the author can edit this PR".into(),
            ));
        }
        if let Some(t) = title {
            let t = t.trim().to_string();
            if t.is_empty() {
                return Err(GitError::InvalidArgument("title required".into()));
            }
            pr.title = t;
        }
        if let Some(b) = body {
            pr.body = b;
        }
        pr.updated_at = Utc::now();
        self.store.update_metadata(&pr).await?;
        Ok(pr)
    }

    pub async fn close(&self, id: PullRequestId, actor_id: PrincipalId) -> Result<PullRequest> {
        let mut pr = self.get(id).await?;
        if pr.status != PullRequestStatus::Open {
            return Err(GitError::Conflict("PR is not open".into()));
        }
        // Author or (later) maintainer
        if pr.author_id != actor_id {
            // Allow any authenticated member for now via repo existence; tighten later
        }
        let now = Utc::now();
        pr.status = PullRequestStatus::Closed;
        pr.closed_at = Some(now);
        pr.updated_at = now;
        self.store.update_status(&pr).await?;
        info!(pr_id = %pr.id, number = pr.number, "pull request closed");
        Ok(pr)
    }

    pub async fn reopen(&self, id: PullRequestId, actor_id: PrincipalId) -> Result<PullRequest> {
        let mut pr = self.get(id).await?;
        if pr.status != PullRequestStatus::Closed {
            return Err(GitError::Conflict("only closed PRs can be reopened".into()));
        }
        let _ = actor_id;
        let now = Utc::now();
        pr.status = PullRequestStatus::Open;
        pr.closed_at = None;
        pr.updated_at = now;
        self.store.update_status(&pr).await?;
        info!(pr_id = %pr.id, number = pr.number, "pull request reopened");
        Ok(pr)
    }

    /// Merge source into target on the bare repo, then mark PR merged.
    pub async fn merge(
        &self,
        id: PullRequestId,
        actor_id: PrincipalId,
        requested_method: Option<MergeMethod>,
    ) -> Result<PullRequest> {
        let mut pr = self.get(id).await?;
        if pr.status != PullRequestStatus::Open {
            return Err(GitError::Conflict("only open PRs can be merged".into()));
        }
        let repo = self.repos.get(pr.repository_id).await?;
        let path = self.repos.disk_path(repo.workspace_id, &repo.name);
        if !path.exists() {
            return Err(GitError::Git(format!(
                "bare repo missing on disk: {}",
                path.display()
            )));
        }

        let settings = self.repos.get_settings(pr.repository_id).await?;
        let method = settings
            .resolve_method(requested_method)
            .map_err(GitError::InvalidArgument)?;

        // --- Protections ---
        if settings.restrict_merge_to_admins && repo.created_by != actor_id {
            return Err(GitError::PermissionDenied(
                "only repository admins may merge pull requests".into(),
            ));
        }

        let report = crate::protect::inspect_branches(
            &path,
            &pr.source_branch,
            &pr.target_branch,
        )
        .await?;

        if settings.require_up_to_date && !report.is_up_to_date {
            return Err(GitError::Conflict(format!(
                "branch is behind target by {} commit(s); update branch before merging",
                report.behind_by
            )));
        }

        if report.has_conflicts {
            return Err(GitError::Conflict(format!(
                "merge would conflict: {}",
                report
                    .conflict_hint
                    .unwrap_or_else(|| "resolve conflicts first".into())
            )));
        }

        let message = format!(
            "Merge pull request #{} from {}\n\n{}",
            pr.number, pr.source_branch, pr.title
        );
        let outcome = crate::merge::merge_branches(
            &path,
            &pr.source_branch,
            &pr.target_branch,
            &message,
            method,
        )
        .await?;

        if settings.delete_branch_on_merge {
            if let Err(e) = crate::merge::delete_branch(&path, &pr.source_branch).await {
                tracing::warn!(
                    error = %e,
                    branch = %pr.source_branch,
                    "delete_branch_on_merge failed"
                );
            }
        }

        let now = Utc::now();
        pr.status = PullRequestStatus::Merged;
        pr.merged_by = Some(actor_id);
        pr.merged_at = Some(now);
        pr.closed_at = Some(now);
        pr.updated_at = now;
        self.store.update_status(&pr).await?;
        info!(
            pr_id = %pr.id,
            number = pr.number,
            merged_by = %actor_id,
            merge_commit = %outcome.merge_commit,
            method = method.as_str(),
            "pull request merged"
        );
        Ok(pr)
    }


    async fn verify_branches(
        &self,
        repo: &Repository,
        source: &str,
        target: &str,
    ) -> Result<()> {
        let path = self.repos.disk_path(repo.workspace_id, &repo.name);
        if !path.exists() {
            return Ok(()); // soft: metadata-only repos
        }
        let path = path.clone();
        let source = source.to_string();
        let target = target.to_string();
        tokio::task::spawn_blocking(move || {
            for branch in [&source, &target] {
                let refname = format!("refs/heads/{branch}");
                let status = std::process::Command::new("git")
                    .args(["--git-dir"])
                    .arg(&path)
                    .args(["show-ref", "--verify", "--quiet", &refname])
                    .status()
                    .map_err(GitError::Io)?;
                if !status.success() {
                    return Err(GitError::InvalidArgument(format!(
                        "branch not found on server: {branch}"
                    )));
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| GitError::Other(anyhow::anyhow!("join: {e}")))?
    }
}

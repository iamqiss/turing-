//! Repository metadata + on-disk bare repos.

mod store;
mod settings;

pub use store::{PostgresRepoStore, RepoStore};
pub use settings::{PostgresSettingsStore, SettingsStore};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use tt_core::domain::Repository;
use tt_core::ids::{PrincipalId, RepositoryId, WorkspaceId};

use crate::error::{GitError, Result};

/// Validates repo name: lowercase alphanumeric, hyphens, underscores; 1–100 chars.
pub fn validate_repo_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 100 {
        return Err(GitError::InvalidName(
            "name must be 1–100 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(GitError::InvalidName(
            "use lowercase letters, digits, hyphens, underscores only".into(),
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(GitError::InvalidName(
            "cannot start or end with a hyphen".into(),
        ));
    }
    Ok(())
}

/// On-disk layout: `{root}/{workspace_id}/{name}.git`
pub fn repo_disk_path(root: &Path, workspace_id: WorkspaceId, name: &str) -> PathBuf {
    root.join(workspace_id.to_string())
        .join(format!("{name}.git"))
}

fn absolute_path(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::fs::canonicalize(p) {
        Ok(c) => c,
        Err(_) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            cwd.join(p)
        }
    }
}

pub struct RepoService {
    store: Arc<dyn RepoStore>,
    settings: Arc<dyn SettingsStore>,
    repos_root: PathBuf,
}

impl RepoService {
    pub fn new(
        store: Arc<dyn RepoStore>,
        settings: Arc<dyn SettingsStore>,
        repos_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            store,
            settings,
            repos_root: repos_root.into(),
        }
    }

    pub async fn get_settings(&self, repository_id: RepositoryId) -> Result<tt_core::domain::RepositorySettings> {
        // Ensure repo exists
        let _ = self.get(repository_id).await?;
        self.settings.get(repository_id).await
    }

    pub async fn update_settings(
        &self,
        settings: tt_core::domain::RepositorySettings,
    ) -> Result<tt_core::domain::RepositorySettings> {
        let _ = self.get(settings.repository_id).await?;
        self.settings.upsert(&settings).await?;
        self.settings.get(settings.repository_id).await
    }

    pub fn repos_root(&self) -> &Path {
        &self.repos_root
    }

    pub fn disk_path(&self, workspace_id: WorkspaceId, name: &str) -> PathBuf {
        let p = repo_disk_path(&self.repos_root, workspace_id, name);
        absolute_path(&p)
    }

    /// Absolute repos root (for Smart HTTP and tooling).
    pub fn absolute_repos_root(&self) -> PathBuf {
        absolute_path(&self.repos_root)
    }

    /// Create metadata row + bare repository on disk.
    pub async fn create(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
        description: Option<String>,
        is_private: bool,
        created_by: PrincipalId,
        default_branch: &str,
    ) -> Result<Repository> {
        validate_repo_name(name)?;

        if self
            .store
            .get_by_name(workspace_id, name)
            .await?
            .is_some()
        {
            return Err(GitError::AlreadyExists);
        }

        let id = RepositoryId::new();
        let now = Utc::now();
        let repo = Repository {
            id,
            workspace_id,
            name: name.to_string(),
            description,
            default_branch: default_branch.to_string(),
            is_private,
            created_by,
            created_at: now,
            updated_at: now,
        };

        // Init bare on disk first so we don't leave orphan DB rows if disk fails
        let path = self.disk_path(workspace_id, name);
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        init_bare_repo(&path, default_branch).await?;

        self.store.create(&repo).await?;
        info!(%id, %workspace_id, name, path = %path.display(), "repository created");
        Ok(repo)
    }

    pub async fn get(&self, id: RepositoryId) -> Result<Repository> {
        self.store
            .get(id)
            .await?
            .ok_or(GitError::NotFound)
    }

    pub async fn get_by_name(&self, workspace_id: WorkspaceId, name: &str) -> Result<Repository> {
        self.store
            .get_by_name(workspace_id, name)
            .await?
            .ok_or(GitError::NotFound)
    }

    pub async fn list_for_workspace(&self, workspace_id: WorkspaceId) -> Result<Vec<Repository>> {
        self.store.list_by_workspace(workspace_id).await
    }

    pub async fn delete(&self, id: RepositoryId) -> Result<()> {
        let repo = self.get(id).await?;
        let path = self.disk_path(repo.workspace_id, &repo.name);
        self.store.delete(id).await?;
        if path.exists() {
            tokio::fs::remove_dir_all(&path).await?;
        }
        info!(%id, "repository deleted");
        Ok(())
    }

    /// Resolve on-disk path for an existing repo.
    pub async fn path_for(&self, id: RepositoryId) -> Result<PathBuf> {
        let repo = self.get(id).await?;
        let path = self.disk_path(repo.workspace_id, &repo.name);
        if !path.exists() {
            return Err(GitError::Git(format!(
                "bare repo missing on disk: {}",
                path.display()
            )));
        }
        Ok(path)
    }
}

/// Create a bare repository with the given default branch (symbolic HEAD).
async fn init_bare_repo(path: &Path, default_branch: &str) -> Result<()> {
    let path = path.to_path_buf();
    let branch = default_branch.to_string();
    tokio::task::spawn_blocking(move || {
        // Prefer gix; fall back to `git init --bare` if the API differs.
        let init_result = gix::init_bare(&path);
        if let Err(e) = init_result {
            // Fallback
            let status = std::process::Command::new("git")
                .args(["init", "--bare", "-b", &branch])
                .arg(&path)
                .status()
                .map_err(GitError::Io)?;
            if !status.success() {
                return Err(GitError::Git(format!(
                    "gix init_bare failed ({e}); git init --bare also failed"
                )));
            }
        }

        let head_path = path.join("HEAD");
        std::fs::write(&head_path, format!("ref: refs/heads/{branch}\n"))
            .map_err(GitError::Io)?;
        let _ = std::fs::create_dir_all(path.join("refs/heads"));
        let _ = std::fs::create_dir_all(path.join("objects/pack"));
        Ok::<(), GitError>(())
    })
    .await
    .map_err(|e| GitError::Other(anyhow::anyhow!("join: {e}")))?
}

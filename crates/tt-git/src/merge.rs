//! Server-side branch merge on bare repositories (merge / squash / rebase).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tracing::{info, warn};
use tt_core::domain::MergeMethod;

use crate::error::{GitError, Result};

pub struct MergeOutcome {
    pub merge_commit: String,
    pub method: MergeMethod,
}

pub async fn merge_branches(
    git_dir: &Path,
    source_branch: &str,
    target_branch: &str,
    message: &str,
    method: MergeMethod,
) -> Result<MergeOutcome> {
    let git_dir = git_dir.to_path_buf();
    let source = source_branch.to_string();
    let target = target_branch.to_string();
    let message = message.to_string();

    tokio::task::spawn_blocking(move || {
        merge_branches_sync(&git_dir, &source, &target, &message, method)
    })
    .await
    .map_err(|e| GitError::Other(anyhow::anyhow!("merge join: {e}")))?
}

fn merge_branches_sync(
    git_dir: &Path,
    source: &str,
    target: &str,
    message: &str,
    method: MergeMethod,
) -> Result<MergeOutcome> {
    let git_dir_buf = canonicalize_git_dir(git_dir)?;
    let git_dir = git_dir_buf.as_path();

    if !git_dir.exists() {
        return Err(GitError::Git(format!(
            "bare repo missing: {}",
            git_dir.display()
        )));
    }
    if !git_dir.join("HEAD").exists() && !git_dir.join("objects").exists() {
        return Err(GitError::Git(format!(
            "path exists but is not a git repo: {}",
            git_dir.display()
        )));
    }

    verify_ref(git_dir, source)?;
    verify_ref(git_dir, target)?;

    let wt = tempfile_worktree_path(git_dir)?;
    if let Some(parent) = wt.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _ = run_git(git_dir, &["worktree", "prune"]);

    let add = run_git(
        git_dir,
        &[
            "worktree",
            "add",
            "--force",
            "-B",
            target,
            wt.to_str().unwrap(),
            target,
        ],
    );
    if let Err(e) = add {
        let _ = run_git(git_dir, &["worktree", "prune"]);
        return Err(e);
    }

    let result = (|| {
        run_git_in(&wt, git_dir, &["reset", "--hard", "HEAD"])?;
        run_git_in(&wt, git_dir, &["clean", "-fd"])?;

        match method {
            MergeMethod::Merge => {
                run_git_in(
                    &wt,
                    git_dir,
                    &["merge", "--no-ff", "--no-edit", "-m", message, source],
                )?;
            }
            MergeMethod::Squash => {
                run_git_in(&wt, git_dir, &["merge", "--squash", source])?;
                // Commit the index as a single commit on target
                run_git_in(
                    &wt,
                    git_dir,
                    &["commit", "--no-edit", "-m", message],
                )?;
            }
            MergeMethod::Rebase => {
                // Rebase source onto target in a detached sense: checkout source, rebase onto target,
                // then fast-forward target.
                // Safer pattern for bare+worktree:
                // 1. worktree is on target (done)
                // 2. merge --ff-only after rebase of source onto target in a second worktree is heavy.
                // Practical approach: checkout -B temp source, rebase onto target, then
                // reset target to temp tip with ff-only from target worktree.
                let temp = format!("tt-rebase-{}", std::process::id());
                run_git_in(
                    &wt,
                    git_dir,
                    &["checkout", "-B", &temp, source],
                )?;
                run_git_in(&wt, git_dir, &["rebase", target]).map_err(|e| {
                    let _ = run_git_in(&wt, git_dir, &["rebase", "--abort"]);
                    e
                })?;
                // Move target to rebased tip
                let tip = run_git_output_in(&wt, git_dir, &["rev-parse", "HEAD"])?
                    .trim()
                    .to_string();
                run_git_in(&wt, git_dir, &["checkout", target])?;
                run_git_in(&wt, git_dir, &["reset", "--hard", &tip])?;
                // Drop temp branch ref
                let _ = run_git(git_dir, &["branch", "-D", &temp]);
            }
        }

        let sha = run_git_output(git_dir, &["rev-parse", target])?
            .trim()
            .to_string();
        Ok(MergeOutcome {
            merge_commit: sha,
            method,
        })
    })();

    if let Err(e) = run_git(
        git_dir,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    ) {
        warn!(error = %e, "worktree remove failed; pruning");
        let _ = run_git(git_dir, &["worktree", "prune"]);
        let _ = std::fs::remove_dir_all(&wt);
    }

    match &result {
        Ok(o) => info!(
            target,
            source,
            method = o.method.as_str(),
            commit = %o.merge_commit,
            "server-side merge succeeded"
        ),
        Err(e) => warn!(target, source, error = %e, "server-side merge failed"),
    }
    result
}

/// Optionally delete the source branch ref after a successful merge.
pub async fn delete_branch(git_dir: &Path, branch: &str) -> Result<()> {
    let git_dir = git_dir.to_path_buf();
    let branch = branch.to_string();
    tokio::task::spawn_blocking(move || {
        let git_dir = canonicalize_git_dir(&git_dir)?;
        // Refuse to delete HEAD branch
        let head = run_git_output(&git_dir, &["symbolic-ref", "--short", "HEAD"])
            .unwrap_or_default()
            .trim()
            .to_string();
        if head == branch {
            return Err(GitError::InvalidArgument(
                "cannot delete the current HEAD branch".into(),
            ));
        }
        run_git(&git_dir, &["branch", "-D", &branch])
    })
    .await
    .map_err(|e| GitError::Other(anyhow::anyhow!("delete branch join: {e}")))?
}

fn canonicalize_git_dir(git_dir: &Path) -> Result<PathBuf> {
    if git_dir.is_absolute() {
        return Ok(git_dir.to_path_buf());
    }
    match std::fs::canonicalize(git_dir) {
        Ok(p) => Ok(p),
        Err(_) => Ok(std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(git_dir)),
    }
}

fn verify_ref(git_dir: &Path, branch: &str) -> Result<()> {
    let refname = format!("refs/heads/{branch}");
    let status = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["show-ref", "--verify", "--quiet", &refname])
        .status()
        .map_err(GitError::Io)?;
    if !status.success() {
        return Err(GitError::InvalidArgument(format!(
            "branch not found: {branch}"
        )));
    }
    Ok(())
}

fn tempfile_worktree_path(git_dir: &Path) -> Result<PathBuf> {
    let base = std::env::temp_dir().join("tt-merge");
    std::fs::create_dir_all(&base)?;
    let name = format!(
        "{}-{}",
        git_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("repo"),
        std::process::id()
    );
    Ok(base.join(name))
}

fn apply_identity(cmd: &mut std::process::Command) {
    cmd.env("GIT_AUTHOR_NAME", "Turing Tarpit")
        .env("GIT_AUTHOR_EMAIL", "merge@turing-tarpit.local")
        .env("GIT_COMMITTER_NAME", "Turing Tarpit")
        .env("GIT_COMMITTER_EMAIL", "merge@turing-tarpit.local")
        .env("GIT_TERMINAL_PROMPT", "0");
}

fn run_git(git_dir: &Path, args: &[&str]) -> Result<()> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("--git-dir")
        .arg(git_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_identity(&mut cmd);
    let out = cmd.output().map_err(GitError::Io)?;
    if out.status.success() {
        return Ok(());
    }
    Err(classify_git_failure(&out.stdout, &out.stderr))
}

fn run_git_in(work_tree: &Path, git_dir: &Path, args: &[&str]) -> Result<()> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(work_tree)
        .args(args)
        .current_dir(work_tree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_identity(&mut cmd);
    let out = cmd.output().map_err(GitError::Io)?;
    if out.status.success() {
        return Ok(());
    }
    Err(classify_git_failure(&out.stdout, &out.stderr))
}

fn run_git_output(git_dir: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("--git-dir")
        .arg(git_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_identity(&mut cmd);
    let out = cmd.output().map_err(GitError::Io)?;
    if !out.status.success() {
        return Err(classify_git_failure(&out.stdout, &out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run_git_output_in(work_tree: &Path, git_dir: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(work_tree)
        .args(args)
        .current_dir(work_tree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_identity(&mut cmd);
    let out = cmd.output().map_err(GitError::Io)?;
    if !out.status.success() {
        return Err(classify_git_failure(&out.stdout, &out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn classify_git_failure(stdout: &[u8], stderr: &[u8]) -> GitError {
    let stderr = String::from_utf8_lossy(stderr);
    let stdout = String::from_utf8_lossy(stdout);
    let msg = if stderr.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        stderr.trim().to_string()
    };
    let lower = msg.to_lowercase();
    if lower.contains("conflict")
        || lower.contains("automatic merge failed")
        || lower.contains("could not apply")
        || lower.contains("would be overwritten")
    {
        GitError::Conflict(format!("merge conflict: {msg}"))
    } else {
        GitError::Git(msg)
    }
}

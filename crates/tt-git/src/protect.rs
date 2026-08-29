//! Pre-merge protection checks against a bare repository.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::{GitError, Result};

#[derive(Debug)]
pub struct ProtectionReport {
    pub source_sha: String,
    pub target_sha: String,
    pub behind_by: u32,
    pub ahead_by: u32,
    pub is_up_to_date: bool,
    pub has_conflicts: bool,
    pub conflict_hint: Option<String>,
}

pub async fn inspect_branches(
    git_dir: &Path,
    source: &str,
    target: &str,
) -> Result<ProtectionReport> {
    let git_dir = git_dir.to_path_buf();
    let source = source.to_string();
    let target = target.to_string();
    tokio::task::spawn_blocking(move || inspect_sync(&git_dir, &source, &target))
        .await
        .map_err(|e| GitError::Other(anyhow::anyhow!("protect join: {e}")))?
}

fn canonicalize(git_dir: &Path) -> PathBuf {
    if git_dir.is_absolute() {
        return git_dir.to_path_buf();
    }
    std::fs::canonicalize(git_dir).unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(git_dir)
    })
}

fn inspect_sync(git_dir: &Path, source: &str, target: &str) -> Result<ProtectionReport> {
    let git_dir = canonicalize(git_dir);
    let source_sha = rev_parse(&git_dir, source)?;
    let target_sha = rev_parse(&git_dir, target)?;

    // How many commits on target not in source (behind), and source not in target (ahead)
    let behind = rev_list_count(&git_dir, &format!("{source}..{target}"))?;
    let ahead = rev_list_count(&git_dir, &format!("{target}..{source}"))?;
    let is_up_to_date = behind == 0;

    let (has_conflicts, conflict_hint) = merge_tree_conflicts(&git_dir, source, target)?;

    Ok(ProtectionReport {
        source_sha,
        target_sha,
        behind_by: behind,
        ahead_by: ahead,
        is_up_to_date,
        has_conflicts,
        conflict_hint,
    })
}

fn rev_parse(git_dir: &Path, rev: &str) -> Result<String> {
    let out = git_output(git_dir, &["rev-parse", rev])?;
    Ok(out.trim().to_string())
}

fn rev_list_count(git_dir: &Path, range: &str) -> Result<u32> {
    let out = git_output(git_dir, &["rev-list", "--count", range])?;
    out.trim()
        .parse()
        .map_err(|_| GitError::Git(format!("bad rev-list count for {range}")))
}

/// Use `git merge-tree` when available; fall back to a temporary index merge simulation.
fn merge_tree_conflicts(
    git_dir: &Path,
    source: &str,
    target: &str,
) -> Result<(bool, Option<String>)> {
    // git merge-tree $(git merge-base A B) A B  — older form
    // git merge-tree --write-tree -z target source — newer
    let base = match git_output(git_dir, &["merge-base", target, source]) {
        Ok(b) => b.trim().to_string(),
        Err(_) => {
            // Unrelated histories
            return Ok((true, Some("no merge-base (unrelated histories)".into())));
        }
    };

    let out = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["merge-tree", &base, target, source])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(GitError::Io)?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Conflict markers in merge-tree output: lines with +<<<<<<< or "CONFLICT"
    let combined = format!("{stdout}\n{stderr}");
    let has = combined.contains("changed in both")
        || combined.contains("CONFLICT")
        || combined.contains("<<<<<<<");
    let hint = if has {
        Some(
            combined
                .lines()
                .find(|l| l.contains("CONFLICT") || l.contains("changed in both"))
                .unwrap_or("merge would conflict")
                .to_string(),
        )
    } else {
        None
    };
    Ok((has, hint))
}

fn git_output(git_dir: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(GitError::Io)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(GitError::Git(stderr.trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

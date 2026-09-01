//! Git hooks — Scarlet Letter push protection (secret-shaped strings).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tracing::{info, warn};
use tt_secrets::{apply_allowlist, scan_text, ScanReport};

use crate::error::{GitError, Result};

const ZERO: &str = "0000000000000000000000000000000000000000";

/// Scan all commits in `old_sha..new_sha` (or all reachable from new if old is zero).
pub async fn scan_push_range(
    git_dir: &Path,
    old_sha: &str,
    new_sha: &str,
    allowlist: &[String],
) -> Result<ScanReport> {
    let git_dir = git_dir.to_path_buf();
    let old = old_sha.to_string();
    let new = new_sha.to_string();
    let allow = allowlist.to_vec();
    tokio::task::spawn_blocking(move || scan_range_sync(&git_dir, &old, &new, &allow))
        .await
        .map_err(|e| GitError::Other(anyhow::anyhow!("scan join: {e}")))?
}

fn scan_range_sync(
    git_dir: &Path,
    old_sha: &str,
    new_sha: &str,
    allowlist: &[String],
) -> Result<ScanReport> {
    if new_sha == ZERO {
        return Ok(ScanReport {
            findings: vec![],
            scanned_bytes: 0,
            blocked: false,
        });
    }

    let range = if old_sha == ZERO || old_sha.is_empty() {
        new_sha.to_string()
    } else {
        format!("{old_sha}..{new_sha}")
    };

    // List new commits (cap to avoid DoS)
    let commits = git_lines(git_dir, &["rev-list", "--max-count=50", &range])?;
    let mut combined = String::new();
    for c in commits {
        if c.trim().is_empty() {
            continue;
        }
        // Patch for each commit (binary safe-ish via -a text)
        if let Ok(patch) = git_output(
            git_dir,
            &["show", "--pretty=format:%B", "--no-color", "-p", "--", c.trim()],
        ) {
            // Cap size per commit
            if patch.len() > 2_000_000 {
                combined.push_str(&patch[..2_000_000]);
            } else {
                combined.push_str(&patch);
            }
            combined.push('\n');
        }
    }

    let report = scan_text(&combined);
    Ok(apply_allowlist(report, allowlist))
}

fn git_output(git_dir: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(["--git-dir"])
        .arg(git_dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| GitError::Git(format!("git: {e}")))?;
    if !out.status.success() {
        return Err(GitError::Git(String::from_utf8_lossy(&out.stderr).into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_lines(git_dir: &Path, args: &[&str]) -> Result<Vec<String>> {
    Ok(git_output(git_dir, args)?
        .lines()
        .map(|s| s.to_string())
        .collect())
}

/// Install a pre-receive hook that rejects secret-shaped content.
/// The hook invokes `git` only (no TT binary required on PATH).
pub async fn install_push_protection_hook(git_dir: &Path) -> Result<()> {
    let hooks = git_dir.join("hooks");
    tokio::fs::create_dir_all(&hooks)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
    let path = hooks.join("pre-receive");
    let script = PRE_RECEIVE_HOOK;
    tokio::fs::write(&path, script)
        .await
        .map_err(|e| GitError::Other(e.into()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&path)
            .await
            .map_err(|e| GitError::Other(e.into()))?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&path, perms)
            .await
            .map_err(|e| GitError::Other(e.into()))?;
    }
    info!(path = %path.display(), "installed Scarlet Letter pre-receive hook");
    Ok(())
}

/// Ensure hook exists (idempotent).
pub async fn ensure_protection_hook(git_dir: &Path) -> Result<()> {
    let path = git_dir.join("hooks/pre-receive");
    if path.exists() {
        // Refresh if marker missing
        if let Ok(body) = tokio::fs::read_to_string(&path).await {
            if body.contains("SCARLET_LETTER_PUSH_PROTECTION") {
                return Ok(());
            }
        }
    }
    install_push_protection_hook(git_dir).await
}

const PRE_RECEIVE_HOOK: &str = r#"#!/bin/sh
# SCARLET_LETTER_PUSH_PROTECTION — Turing Tarpit
# Rejects pushes containing secret-shaped strings in new commits.
set -e
zero=0000000000000000000000000000000000000000
# Patterns (keep in sync with tt-secrets::scan)
pat_key='-----BEGIN .*PRIVATE KEY-----'
pat_akia='AKIA[0-9A-Z]{16}'
pat_gh='gh[pousr]_[A-Za-z0-9_]{36,}'
pat_stripe='sk_(live|test)_[0-9a-zA-Z]{20,}'
pat_conn='(postgres|mysql|mongodb|redis)://[^[:space:]]{8,}'
pat_tt='tt_deploy_[A-Za-z0-9_]+_[A-Za-z0-9_\-]{16,}'

scan_blob() {
  f="$1"
  if grep -aEq "$pat_key|$pat_akia|$pat_gh|$pat_stripe|$pat_conn|$pat_tt" "$f" 2>/dev/null; then
    return 1
  fi
  # password= / secret= assignments with long values
  if grep -aEi '(password|secret|api[_-]?key|token)\s*[=:]\s*.{12,}' "$f" 2>/dev/null | grep -aEq '.{20,}'; then
    return 1
  fi
  return 0
}

while read -r oldrev newrev refname; do
  [ "$newrev" = "$zero" ] && continue
  if [ "$oldrev" = "$zero" ]; then
    revs=$(git rev-list --max-count=50 "$newrev")
  else
    revs=$(git rev-list --max-count=50 "$oldrev..$newrev")
  fi
  for rev in $revs; do
    tmp=$(mktemp)
    # Commit message + patch (text)
    git show --pretty=format:%B --no-color -p "$rev" 2>/dev/null | head -c 2000000 >"$tmp" || true
    if ! scan_blob "$tmp"; then
      echo "remote: Scarlet Letter ✉ blocked push: secret-shaped string in commit $rev" >&2
      echo "remote: Refusing update to $refname" >&2
      echo "remote: Remove the secret, use Scarlet Letter store, then push again." >&2
      rm -f "$tmp"
      exit 1
    fi
    rm -f "$tmp"
  done
done
exit 0
"#;

/// Client-side pre-commit helper text (install into .git/hooks/pre-commit).
pub fn pre_commit_hook_script() -> &'static str {
    r#"#!/bin/sh
# Turing Tarpit — Scarlet Letter pre-commit (local)
# Install: tt secrets install-hook   or copy to .git/hooks/pre-commit
set -e
pat_key='-----BEGIN .*PRIVATE KEY-----'
pat_akia='AKIA[0-9A-Z]{16}'
pat_gh='gh[pousr]_[A-Za-z0-9_]{36,}'
pat_stripe='sk_(live|test)_[0-9a-zA-Z]{20,}'
pat_conn='(postgres|mysql|mongodb|redis)://[^[:space:]]{8,}'

# Staged files only
files=$(git diff --cached --name-only --diff-filter=ACM)
[ -z "$files" ] && exit 0
tmp=$(mktemp)
for f in $files; do
  git show ":$f" 2>/dev/null >>"$tmp" || true
done
if grep -aEq "$pat_key|$pat_akia|$pat_gh|$pat_stripe|$pat_conn" "$tmp" 2>/dev/null; then
  echo "Scarlet Letter ✉: secret-shaped string in staged files. Commit blocked." >&2
  echo "Store secrets with Scarlet Letter — never commit .env or keys." >&2
  rm -f "$tmp"
  exit 1
fi
if grep -aEi '(password|secret|api[_-]?key)\s*[=:]\s*.{12,}' "$tmp" 2>/dev/null | head -1 | grep -q .; then
  echo "Scarlet Letter ✉: possible secret assignment in staged files." >&2
  rm -f "$tmp"
  exit 1
fi
rm -f "$tmp"
exit 0
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delete_ref_not_scanned() {
        let report = scan_push_range(Path::new("."), ZERO, ZERO, &[]).await.unwrap();
        // new==zero means delete; our code checks new_sha == ZERO first
        assert!(!report.blocked);
    }
}

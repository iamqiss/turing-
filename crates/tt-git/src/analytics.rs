//! Post-push analysis: languages, density, commit list.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;

use chrono::{TimeZone, Utc};
use tracing::warn;
use tt_core::ids::{PrincipalId, RepositoryId, WorkspaceId};

use crate::error::{GitError, Result};

#[derive(Debug, Clone)]
pub struct PushAnalysis {
    pub workspace_id: WorkspaceId,
    pub repository_id: RepositoryId,
    pub principal_id: PrincipalId,
    pub commits: Vec<CommitStat>,
}

#[derive(Debug, Clone)]
pub struct CommitStat {
    pub sha: String,
    pub files_touched: u32,
    pub lines_added: u32,
    pub lines_deleted: u32,
    pub languages: Vec<String>,
    pub message_summary: String,
    pub ts: chrono::DateTime<Utc>,
}

/// Analyze the last `limit` commits on all branches (best-effort after a push).
pub async fn analyze_recent_commits(
    git_dir: &Path,
    workspace_id: WorkspaceId,
    repository_id: RepositoryId,
    principal_id: PrincipalId,
    limit: usize,
) -> Result<PushAnalysis> {
    let git_dir = git_dir.to_path_buf();
    let limit = limit.clamp(1, 50);
    tokio::task::spawn_blocking(move || {
        analyze_sync(&git_dir, workspace_id, repository_id, principal_id, limit)
    })
    .await
    .map_err(|e| GitError::Other(anyhow::anyhow!("analytics join: {e}")))?
}

fn analyze_sync(
    git_dir: &Path,
    workspace_id: WorkspaceId,
    repository_id: RepositoryId,
    principal_id: PrincipalId,
    limit: usize,
) -> Result<PushAnalysis> {
    if !git_dir.exists() {
        return Ok(PushAnalysis {
            workspace_id,
            repository_id,
            principal_id,
            commits: vec![],
        });
    }

    // sha | timestamp | subject
    let log = git_output(
        git_dir,
        &[
            "log",
            "--all",
            &format!("-n{limit}"),
            "--format=%H%x09%ct%x09%s",
        ],
    )
    .unwrap_or_default();

    let mut commits = Vec::new();
    for line in log.lines() {
        let mut parts = line.splitn(3, '\t');
        let sha = parts.next().unwrap_or("").to_string();
        let ct: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let subject = parts.next().unwrap_or("").to_string();
        if sha.is_empty() {
            continue;
        }
        match commit_stat(git_dir, &sha, &subject, ct) {
            Ok(c) => commits.push(c),
            Err(e) => warn!(error = %e, %sha, "commit stat failed"),
        }
    }

    Ok(PushAnalysis {
        workspace_id,
        repository_id,
        principal_id,
        commits,
    })
}

fn commit_stat(git_dir: &Path, sha: &str, subject: &str, ct: i64) -> Result<CommitStat> {
    // numstat: added deleted path
    let numstat = git_output(git_dir, &["show", "--numstat", "--format=", sha]).unwrap_or_default();
    let mut files = 0u32;
    let mut added = 0u32;
    let mut deleted = 0u32;
    let mut lang_counts: HashMap<String, u32> = HashMap::new();

    for line in numstat.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let a = parts.next().unwrap_or("0");
        let d = parts.next().unwrap_or("0");
        let path = parts.next().unwrap_or("");
        if path.is_empty() {
            continue;
        }
        files += 1;
        if a != "-" {
            added += a.parse().unwrap_or(0);
        }
        if d != "-" {
            deleted += d.parse().unwrap_or(0);
        }
        if let Some(lang) = language_from_path(path) {
            *lang_counts.entry(lang).or_insert(0) += 1;
        }
    }

    let mut languages: Vec<_> = lang_counts.into_iter().collect();
    languages.sort_by(|a, b| b.1.cmp(&a.1));
    let languages: Vec<String> = languages.into_iter().map(|(l, _)| l).take(5).collect();

    let ts = Utc
        .timestamp_opt(ct, 0)
        .single()
        .unwrap_or_else(Utc::now);

    Ok(CommitStat {
        sha: sha.to_string(),
        files_touched: files,
        lines_added: added,
        lines_deleted: deleted,
        languages,
        message_summary: subject.chars().take(120).collect(),
        ts,
    })
}

pub fn language_from_path(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    // special filenames
    let special = match name {
        "Cargo.toml" | "Cargo.lock" | "rust-toolchain" | "rust-toolchain.toml" => Some("Rust"),
        "go.mod" | "go.sum" => Some("Go"),
        "package.json" | "tsconfig.json" => Some("TypeScript"),
        "pyproject.toml" | "requirements.txt" | "setup.py" => Some("Python"),
        "Gemfile" => Some("Ruby"),
        "pom.xml" | "build.gradle" | "build.gradle.kts" => Some("Java"),
        "CMakeLists.txt" => Some("C++"),
        "Dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => Some("Docker"),
        "Makefile" => Some("Make"),
        _ => None,
    };
    if special.is_some() {
        return special.map(|s| s.to_string());
    }
    let ext = name.rsplit('.').next()?.to_lowercase();
    let lang = match ext.as_str() {
        "rs" => "Rust",
        "go" => "Go",
        "py" | "pyi" => "Python",
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "java" | "kt" | "kts" => "Java",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" => "C++",
        "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "cs" => "C#",
        "scala" => "Scala",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "md" | "mdx" => "Markdown",
        "toml" => "Toml",
        "yaml" | "yml" => "Yaml",
        "json" => "Json",
        "html" | "htm" => "HTML",
        "css" | "scss" => "CSS",
        "proto" => "Protobuf",
        "vue" | "svelte" => "Frontend",
        _ => return None,
    };
    Some(lang.to_string())
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

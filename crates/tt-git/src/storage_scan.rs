//! Periodic git on-disk size scan → billing `git.storage_gb_month`.

use std::path::{Path, PathBuf};

use tracing::{info, warn};
use tt_core::ids::{OrgId, WorkspaceId};
use uuid::Uuid;

/// Walk `{repos_root}/{workspace_id}/*.git` and sum bytes (recursive file sizes).
pub fn dir_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let walk = match std::fs::read_dir(path) {
        Ok(w) => w,
        Err(_) => return 0,
    };
    for entry in walk.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total = total.saturating_add(dir_size_bytes(&p));
        } else if let Ok(meta) = entry.metadata() {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

#[derive(Debug, Clone)]
pub struct OrgGitUsage {
    pub org_id: OrgId,
    pub bytes: u64,
    pub repo_count: u32,
}

/// `workspace_org`: pairs of (workspace_id, org_id) for workspaces that belong to an org.
pub fn scan_repos_root(
    repos_root: &Path,
    workspace_orgs: &[(WorkspaceId, OrgId)],
) -> Vec<OrgGitUsage> {
    use std::collections::HashMap;
    let mut map: HashMap<Uuid, (u64, u32)> = HashMap::new();
    for (ws, org) in workspace_orgs {
        let dir = repos_root.join(ws.to_string());
        if !dir.is_dir() {
            continue;
        }
        let mut bytes = 0u64;
        let mut count = 0u32;
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.ends_with(".git") || p.join("HEAD").exists() {
                    bytes = bytes.saturating_add(dir_size_bytes(&p));
                    count += 1;
                }
            }
        }
        let e = map.entry(*org.as_uuid()).or_insert((0, 0));
        e.0 = e.0.saturating_add(bytes);
        e.1 = e.1.saturating_add(count);
    }
    map.into_iter()
        .map(|(id, (bytes, repo_count))| OrgGitUsage {
            org_id: OrgId::from_uuid(id),
            bytes,
            repo_count,
        })
        .collect()
}

pub fn bytes_to_gb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

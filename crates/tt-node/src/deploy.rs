//! Git clone + rootfs bake for deploy-from-git.
//!
//! Flow:
//!   1. git clone --depth 1 (or fetch sha) into instance/src
//!   2. checkout sha if provided
//!   3. bake: copy base rootfs → instance rootfs, mount loop, rsync src into /app
//!
//! Soft-fails when git/mount unavailable (records deploy.json status).

use std::path::{Path, PathBuf};

use serde_json::json;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Debug)]
pub struct DeployOutcome {
    pub status: String,
    pub src_dir: PathBuf,
    pub note: String,
}

pub fn inject_deploy_token(clone_url: &str, token: Option<&str>) -> String {
    let Some(token) = token else {
        return clone_url.to_string();
    };
    // http://host/path → http://x-access-token:TOKEN@host/path
    if let Some(rest) = clone_url.strip_prefix("https://") {
        format!("https://x-access-token:{token}@{rest}")
    } else if let Some(rest) = clone_url.strip_prefix("http://") {
        format!("http://x-access-token:{token}@{rest}")
    } else {
        clone_url.to_string()
    }
}

pub async fn stage_git(
    instance_dir: &Path,
    clone_url: &str,
    git_sha: Option<&str>,
    deploy_token: Option<&str>,
) -> DeployOutcome {
    let clone_url = inject_deploy_token(clone_url, deploy_token);
    let clone_url = clone_url.as_str();
    let src = instance_dir.join("src");
    let _ = fs::create_dir_all(instance_dir).await;

    if src.join(".git").exists() || src.join("HEAD").exists() {
        return DeployOutcome {
            status: "already_staged".into(),
            src_dir: src,
            note: "src present".into(),
        };
    }

    // Prefer git binary
    let git_ok = Command::new("git")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !git_ok {
        let _ = fs::write(
            instance_dir.join("deploy.json"),
            serde_json::to_vec_pretty(&json!({
                "clone_url": clone_url,
                "sha": git_sha,
                "status": "pending_no_git",
            }))
            .unwrap(),
        )
        .await;
        return DeployOutcome {
            status: "pending_no_git".into(),
            src_dir: src,
            note: "git not on PATH".into(),
        };
    }

    let mut args = vec![
        "clone".into(),
        "--depth".into(),
        "1".into(),
        clone_url.to_string(),
        src.to_string_lossy().to_string(),
    ];
    // shallow clone of a branch; if only sha, deepen later
    if let Some(sha) = git_sha {
        // clone default then fetch sha
        let _ = args; // use plain clone first
        let out = Command::new("git")
            .args(["clone", "--depth", "50", clone_url])
            .arg(&src)
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {
                let co = Command::new("git")
                    .current_dir(&src)
                    .args(["checkout", sha])
                    .output()
                    .await;
                if let Ok(c) = co {
                    if !c.status.success() {
                        warn!(
                            stderr = %String::from_utf8_lossy(&c.stderr),
                            "git checkout sha failed"
                        );
                    }
                }
            }
            Ok(o) => {
                warn!(stderr = %String::from_utf8_lossy(&o.stderr), "git clone failed");
                let _ = fs::write(
                    instance_dir.join("deploy.json"),
                    serde_json::to_vec_pretty(&json!({
                        "clone_url": clone_url,
                        "sha": git_sha,
                        "status": "clone_failed",
                        "stderr": String::from_utf8_lossy(&o.stderr),
                    }))
                    .unwrap(),
                )
                .await;
                return DeployOutcome {
                    status: "clone_failed".into(),
                    src_dir: src,
                    note: "git clone failed".into(),
                };
            }
            Err(e) => {
                return DeployOutcome {
                    status: "clone_error".into(),
                    src_dir: src,
                    note: e.to_string(),
                };
            }
        }
    } else {
        let out = Command::new("git")
            .args(["clone", "--depth", "1", clone_url])
            .arg(&src)
            .output()
            .await;
        if let Ok(o) = &out {
            if !o.status.success() {
                warn!(stderr = %String::from_utf8_lossy(&o.stderr), "git clone failed");
                let _ = fs::write(
                    instance_dir.join("deploy.json"),
                    serde_json::to_vec_pretty(&json!({
                        "clone_url": clone_url,
                        "status": "clone_failed",
                    }))
                    .unwrap(),
                )
                .await;
                return DeployOutcome {
                    status: "clone_failed".into(),
                    src_dir: src,
                    note: "clone failed".into(),
                };
            }
        }
    }

    info!(%clone_url, src = %src.display(), "git tree staged");
    let _ = fs::write(
        instance_dir.join("deploy.json"),
        serde_json::to_vec_pretty(&json!({
            "clone_url": clone_url,
            "sha": git_sha,
            "status": "cloned",
            "src": src.to_string_lossy(),
        }))
        .unwrap(),
    )
    .await;

    DeployOutcome {
        status: "cloned".into(),
        src_dir: src,
        note: "ok".into(),
    }
}

/// Bake application into a writable rootfs image.
/// Requires: `cp`, optionally `mount`/`umount` for /app inject (root).

pub async fn bake_rootfs(
    base_rootfs: &Path,
    dest_rootfs: &Path,
    src_app: Option<&Path>,
    guest_net: Option<&crate::guest_net::GuestNetConfig>,
) -> Result<(), String> {
    if !base_rootfs.exists() {
        return Err(format!("base rootfs missing: {}", base_rootfs.display()));
    }
    if !dest_rootfs.exists() {
        fs::copy(base_rootfs, dest_rootfs)
            .await
            .map_err(|e| e.to_string())?;
    }
    if src_app.is_none() && guest_net.is_none() {
        return Ok(());
    }

    let mnt = dest_rootfs.with_extension("mnt");
    let _ = fs::create_dir_all(&mnt).await;
    let mount = Command::new("mount")
        .args([
            "-o",
            "loop",
            &dest_rootfs.to_string_lossy(),
            &mnt.to_string_lossy(),
        ])
        .output()
        .await;

    match mount {
        Ok(o) if o.status.success() => {
            if let Some(app) = src_app {
                if app.exists() {
                    let app_dst = mnt.join("app");
                    let _ = fs::create_dir_all(&app_dst).await;
                    let copy = Command::new("cp")
                        .args([
                            "-a",
                            &format!("{}/.", app.display()),
                            &app_dst.to_string_lossy(),
                        ])
                        .output()
                        .await;
                    if let Ok(c) = copy {
                        if c.status.success() {
                            info!("baked /app into rootfs");
                        } else {
                            warn!(stderr = %String::from_utf8_lossy(&c.stderr), "bake cp failed");
                        }
                    }
                }
            }
            if let Some(net) = guest_net {
                net.inject_into_rootfs(&mnt).await;
            }
            let _ = Command::new("umount").arg(&mnt).status().await;
            let _ = fs::remove_dir_all(&mnt).await;
            Ok(())
        }
        _ => {
            warn!("mount loop failed — src/net left on host only");
            let _ = fs::remove_dir_all(&mnt).await;
            Ok(())
        }
    }
}

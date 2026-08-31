//! Build a cloud-init NoCloud CIDATA ISO for the guest.
//!
//! Tries `genisoimage`, then `mkisofs`, then `xorriso`. Soft-fail → no ISO.

use std::path::{Path, PathBuf};

use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

pub async fn build_cidata_iso(instance_dir: &Path) -> Option<PathBuf> {
    let ci = instance_dir.join("cloud-init");
    if !ci.join("user-data").exists() {
        return None;
    }
    let iso = instance_dir.join("cidata.iso");
    if iso.exists() {
        return Some(iso);
    }

    // genisoimage -output cidata.iso -volid cidata -joliet -rock user-data meta-data network-config
    let mut files = vec!["user-data", "meta-data"];
    if ci.join("network-config").exists() {
        files.push("network-config");
    }

    for bin in ["genisoimage", "mkisofs", "xorriso"] {
        let ok = match bin {
            "xorriso" => {
                let mut args = vec![
                    "-as".into(),
                    "mkisofs".into(),
                    "-output".into(),
                    iso.to_string_lossy().to_string(),
                    "-volid".into(),
                    "cidata".into(),
                    "-joliet".into(),
                    "-rock".into(),
                ];
                for f in &files {
                    args.push(ci.join(f).to_string_lossy().to_string());
                }
                Command::new(bin).args(&args).output().await
            }
            _ => {
                let mut args = vec![
                    "-output".into(),
                    iso.to_string_lossy().to_string(),
                    "-volid".into(),
                    "cidata".into(),
                    "-joliet".into(),
                    "-rock".into(),
                ];
                for f in &files {
                    args.push(ci.join(f).to_string_lossy().to_string());
                }
                Command::new(bin).args(&args).current_dir(&ci).output().await
            }
        };
        match ok {
            Ok(o) if o.status.success() && iso.exists() => {
                info!(path = %iso.display(), %bin, "cidata ISO built");
                return Some(iso);
            }
            Ok(o) => {
                warn!(
                    %bin,
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "ISO tool failed"
                );
            }
            Err(_) => continue,
        }
    }
    warn!("no genisoimage/mkisofs/xorriso — skipping cidata ISO");
    None
}

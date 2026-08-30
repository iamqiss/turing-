//! Firecracker microVM driver.
//!
//! Layout per instance:
//!   {data_dir}/instances/{id}/
//!     config.json     — Firecracker machine config
//!     firecracker.pid
//!     api.sock        — FC API unix socket
//!     rootfs.ext4     — copy or hardlink from image store
//!     vmlinux         — kernel (shared path or copy)
//!
//! Requires on the host:
//!   - `firecracker` binary on PATH (or TT_FIRECRACKER_BIN)
//!   - kernel at TT_FC_KERNEL (default: {data_dir}/images/vmlinux)
//!   - rootfs templates under {data_dir}/images/{slug}.ext4

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::json;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use super::{Driver, DriverError, DriverResult, ResourceSpec};

#[derive(Clone)]
pub struct FirecrackerDriver {
    data_dir: PathBuf,
    firecracker_bin: PathBuf,
    kernel_path: PathBuf,
}

impl FirecrackerDriver {
    pub fn new(data_dir: PathBuf) -> Self {
        let firecracker_bin = std::env::var("TT_FIRECRACKER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("firecracker"));
        let kernel_path = std::env::var("TT_FC_KERNEL")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("images/vmlinux"));
        Self {
            data_dir,
            firecracker_bin,
            kernel_path,
        }
    }

    fn instance_dir(&self, instance_id: &str) -> PathBuf {
        self.data_dir.join("instances").join(instance_id)
    }

    fn images_dir(&self) -> PathBuf {
        self.data_dir.join("images")
    }

    fn resolve_rootfs(&self, image_slug: Option<&str>) -> PathBuf {
        let slug = image_slug.unwrap_or("alpine");
        self.images_dir().join(format!("{slug}.ext4"))
    }

    async fn write_config(
        &self,
        dir: &Path,
        resources: &ResourceSpec,
        rootfs: &Path,
        socket: &Path,
    ) -> Result<PathBuf, DriverError> {
        let config_path = dir.join("config.json");
        // Firecracker jailer-less API config (boot-source + drives + machine-config).
        // Network interfaces are left for a follow-up (tap + CNI); microVMs still boot.
        let cfg = json!({
            "boot-source": {
                "kernel_image_path": self.kernel_path.to_string_lossy(),
                "boot_args": "console=ttyS0 reboot=k panic=1 pci=off"
            },
            "drives": [{
                "drive_id": "rootfs",
                "path_on_host": rootfs.to_string_lossy(),
                "is_root_device": true,
                "is_read_only": false
            }],
            "machine-config": {
                "vcpu_count": resources.vcpu.max(1),
                "mem_size_mib": resources.memory_mb.max(128),
                "smt": false
            }
        });
        fs::write(&config_path, serde_json::to_vec_pretty(&cfg).unwrap()).await?;
        // socket path recorded for operators
        let _ = socket;
        Ok(config_path)
    }

    async fn read_pid(&self, dir: &Path) -> Option<u32> {
        let pid_path = dir.join("firecracker.pid");
        let s = fs::read_to_string(pid_path).await.ok()?;
        s.trim().parse().ok()
    }

    async fn write_pid(&self, dir: &Path, pid: u32) -> Result<(), DriverError> {
        fs::write(dir.join("firecracker.pid"), pid.to_string()).await?;
        Ok(())
    }

    fn binary_available(&self) -> bool {
        which(&self.firecracker_bin)
    }
}

fn which(bin: &Path) -> bool {
    if bin.is_absolute() {
        return bin.exists();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(bin).exists())
        })
        .unwrap_or(false)
}

#[async_trait]
impl Driver for FirecrackerDriver {
    fn name(&self) -> &'static str {
        "firecracker"
    }

    async fn boot(
        &self,
        instance_id: &str,
        resources: &ResourceSpec,
        image_slug: Option<&str>,
    ) -> Result<DriverResult, DriverError> {
        let dir = self.instance_dir(instance_id);
        fs::create_dir_all(&dir).await?;

        let rootfs_src = self.resolve_rootfs(image_slug);
        let rootfs_dst = dir.join("rootfs.ext4");
        let socket = dir.join("api.sock");

        // Dry-run / lab mode when assets or binary missing
        if !self.binary_available() {
            warn!(bin = %self.firecracker_bin.display(), "firecracker binary not found — dry-run boot");
            let note = format!(
                "dry-run: install firecracker and set TT_FIRECRACKER_BIN; expected kernel {}",
                self.kernel_path.display()
            );
            fs::write(
                dir.join("status.json"),
                serde_json::to_vec_pretty(&json!({
                    "status": "dry_run_running",
                    "driver": "firecracker",
                    "vcpu": resources.vcpu,
                    "memory_mb": resources.memory_mb,
                }))
                .unwrap(),
            )
            .await?;
            return Ok(DriverResult {
                accepted: true,
                instance_id: instance_id.into(),
                driver: "firecracker".into(),
                action: "boot".into(),
                pid: None,
                socket: None,
                note,
            });
        }

        if !self.kernel_path.exists() {
            return Err(DriverError::MissingAsset(format!(
                "kernel not found at {} (set TT_FC_KERNEL)",
                self.kernel_path.display()
            )));
        }
        if !rootfs_src.exists() {
            return Err(DriverError::MissingAsset(format!(
                "rootfs not found at {} — place {{slug}}.ext4 under {}",
                rootfs_src.display(),
                self.images_dir().display()
            )));
        }

        // Copy rootfs so each instance has a writable disk (hardlink if same fs optional later)
        if !rootfs_dst.exists() {
            info!(from = %rootfs_src.display(), to = %rootfs_dst.display(), "copying rootfs");
            fs::copy(&rootfs_src, &rootfs_dst).await?;
        }

        let _ = fs::remove_file(&socket).await;
        let config = self
            .write_config(&dir, resources, &rootfs_dst, &socket)
            .await?;

        // firecracker --api-sock PATH --config-file PATH
        let mut child = Command::new(&self.firecracker_bin)
            .arg("--api-sock")
            .arg(&socket)
            .arg("--config-file")
            .arg(&config)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                std::fs::File::create(dir.join("firecracker.stdout")).map_err(DriverError::Io)?,
            ))
            .stderr(Stdio::from(
                std::fs::File::create(dir.join("firecracker.stderr")).map_err(DriverError::Io)?,
            ))
            .spawn()
            .map_err(|e| DriverError::Other(format!("spawn firecracker: {e}")))?;

        let pid = child.id().unwrap_or(0);
        self.write_pid(&dir, pid).await?;

        // Detach: drop child without waiting — process keeps running
        std::mem::forget(child);

        info!(%instance_id, %pid, socket = %socket.display(), "firecracker booted");
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "firecracker".into(),
            action: "boot".into(),
            pid: Some(pid),
            socket: Some(socket.to_string_lossy().into()),
            note: "firecracker process started".into(),
        })
    }

    async fn stop(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        let dir = self.instance_dir(instance_id);
        if let Some(pid) = self.read_pid(&dir).await {
            kill_pid(pid);
            let _ = fs::remove_file(dir.join("firecracker.pid")).await;
            info!(%instance_id, %pid, "firecracker stopped");
            Ok(DriverResult {
                accepted: true,
                instance_id: instance_id.into(),
                driver: "firecracker".into(),
                action: "stop".into(),
                pid: Some(pid),
                socket: None,
                note: "sent SIGTERM".into(),
            })
        } else {
            // dry-run cleanup
            let _ = fs::remove_file(dir.join("status.json")).await;
            Ok(DriverResult {
                accepted: true,
                instance_id: instance_id.into(),
                driver: "firecracker".into(),
                action: "stop".into(),
                pid: None,
                socket: None,
                note: "no pid file (dry-run or already stopped)".into(),
            })
        }
    }

    async fn destroy(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        let _ = self.stop(instance_id).await;
        let dir = self.instance_dir(instance_id);
        if dir.exists() {
            fs::remove_dir_all(&dir).await?;
        }
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "firecracker".into(),
            action: "destroy".into(),
            pid: None,
            socket: None,
            note: "instance directory removed".into(),
        })
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            libc_kill(pid as i32, 15); // SIGTERM
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

#[cfg(unix)]
fn libc_kill(pid: i32, sig: i32) {
    // Avoid nix dep by using libc via std::process on failure path
    use std::process::Command;
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let _ = sig;
}

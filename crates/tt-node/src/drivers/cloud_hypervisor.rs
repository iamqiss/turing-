//! Cloud Hypervisor — persistent VPS-class VMs with disk images.
//!
//! Layout:
//!   {data}/instances/{id}/
//!     disk.raw          — sparse disk (qemu-img or truncate)
//!     ch.pid
//!     cloud-hypervisor.log
//!
//! Boot: cloud-hypervisor --cpus --memory --disk --net --kernel (or firmware)
//! Dry-run when binary missing.

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::json;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use super::{Driver, DriverError, DriverRequest, DriverResult, ResourceSpec};
use crate::deploy::{bake_rootfs, stage_git};
use crate::guest_net::GuestNetConfig;
use crate::netfilter::{apply_dnat, clear_dnat, guest_ip_for};

#[derive(Clone)]
pub struct CloudHypervisorDriver {
    data_dir: PathBuf,
    ch_bin: PathBuf,
    kernel_path: PathBuf,
}

impl CloudHypervisorDriver {
    pub fn new(data_dir: PathBuf) -> Self {
        let ch_bin = std::env::var("TT_CH_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("cloud-hypervisor"));
        let kernel_path = std::env::var("TT_CH_KERNEL")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("images/vmlinux"));
        Self {
            data_dir,
            ch_bin,
            kernel_path,
        }
    }

    fn instance_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("instances").join(id)
    }

    fn binary_ok(&self) -> bool {
        let p = &self.ch_bin;
        if p.is_absolute() {
            return p.exists();
        }
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|d| d.join(p).exists()))
            .unwrap_or(false)
    }

    async fn ensure_disk(&self, dir: &PathBuf, disk_gb: u32) -> Result<PathBuf, DriverError> {
        let disk = dir.join("disk.raw");
        if disk.exists() {
            return Ok(disk);
        }
        let bytes = (disk_gb.max(1) as u64) * 1024 * 1024 * 1024;
        // Prefer qemu-img sparse
        let q = Command::new("qemu-img")
            .args([
                "create",
                "-f",
                "raw",
                &disk.to_string_lossy(),
                &format!("{disk_gb}G"),
            ])
            .output()
            .await;
        if q.map(|o| o.status.success()).unwrap_or(false) {
            info!(path = %disk.display(), "disk.raw via qemu-img");
            return Ok(disk);
        }
        // truncate fallback
        let f = std::fs::File::create(&disk).map_err(DriverError::Io)?;
        f.set_len(bytes).map_err(DriverError::Io)?;
        info!(path = %disk.display(), gb = disk_gb, "disk.raw via truncate");
        Ok(disk)
    }
}

#[async_trait]
impl Driver for CloudHypervisorDriver {
    fn name(&self) -> &'static str {
        "cloud_hypervisor"
    }

    async fn boot(
        &self,
        req: &DriverRequest,
        resources: &ResourceSpec,
    ) -> Result<DriverResult, DriverError> {
        let id = &req.instance_id;
        let dir = self.instance_dir(id);
        fs::create_dir_all(&dir).await?;

        let guest_ip = guest_ip_for(id);
        let mut note = String::from("ch");

        if let Some(url) = &req.git_clone_url {
            let o = stage_git(&dir, url, req.git_sha.as_deref(), req.deploy_token.as_deref()).await;
            note = format!("{note}; deploy={}", o.status);
        }

        let disk = self.ensure_disk(&dir, resources.disk_gb).await?;
        // If we have a base rootfs, try to seed disk first boot only
        let base = self
            .data_dir
            .join("images")
            .join(format!("{}.ext4", req.image_slug.as_deref().unwrap_or("ubuntu-24.04")));
        if base.exists() {
            let seed = dir.join("rootfs.ext4");
            if !seed.exists() {
                let _ = fs::copy(&base, &seed).await;
                let src = dir.join("src");
                let gnet = GuestNetConfig::from_instance(id, &guest_ip);
                let _ = gnet.write_cloud_init(&dir).await;
                let _ = bake_rootfs(
                    &base,
                    &seed,
                    if src.exists() {
                        Some(src.as_path())
                    } else {
                        None
                    },
                    Some(&gnet),
                )
                .await;
            }
        }

        let tap = req.tap_device.clone().unwrap_or_else(|| {
            let h = id.replace('-', "");
            format!("ch{}", &h[..h.len().min(8)])
        });

        if let (Some(pip), Some(ps), Some(pe)) =
            (&req.nat_public_ip, req.nat_port_start, req.nat_port_end)
        {
            let _ = apply_dnat(pip, &guest_ip, ps, pe).await;
        }

        let meta = json!({
            "guest_ip": guest_ip,
            "tap": tap,
            "disk": disk.to_string_lossy(),
            "nat_public_ip": req.nat_public_ip,
            "nat_port_start": req.nat_port_start,
            "nat_port_end": req.nat_port_end,
        });
        fs::write(dir.join("network.json"), serde_json::to_vec_pretty(&meta).unwrap()).await?;

        if !self.binary_ok() {
            warn!("cloud-hypervisor binary missing — dry-run");
            fs::write(
                dir.join("status.json"),
                serde_json::to_vec_pretty(&json!({
                    "status": "dry_run_running",
                    "driver": "cloud_hypervisor",
                    "disk": disk.to_string_lossy(),
                    "vcpu": resources.vcpu,
                    "memory_mb": resources.memory_mb,
                }))
                .unwrap(),
            )
            .await?;
            return Ok(DriverResult {
                accepted: true,
                instance_id: id.clone(),
                driver: "cloud_hypervisor".into(),
                action: "boot".into(),
                pid: None,
                socket: None,
                tap_device: Some(tap),
                note: format!("dry-run; {note}"),
            });
        }

        if !self.kernel_path.exists() {
            return Err(DriverError::MissingAsset(format!(
                "CH kernel missing: {}",
                self.kernel_path.display()
            )));
        }

        // cloud-hypervisor --cpus boot={n} --memory size={m}M --disk path=... --kernel ... --cmdline ...
        let cpus = format!("boot={}", resources.vcpu.max(1));
        let mem = format!("size={}M", resources.memory_mb.max(256));
        let disk_arg = format!("path={}", disk.display());
        let mut child = Command::new(&self.ch_bin)
            .args([
                "--cpus",
                &cpus,
                "--memory",
                &mem,
                "--disk",
                &disk_arg,
                "--kernel",
                &self.kernel_path.to_string_lossy(),
                "--cmdline",
                "console=ttyS0 root=/dev/vda rw",
            ])
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                std::fs::File::create(dir.join("ch.stdout")).map_err(DriverError::Io)?,
            ))
            .stderr(Stdio::from(
                std::fs::File::create(dir.join("ch.stderr")).map_err(DriverError::Io)?,
            ))
            .spawn()
            .map_err(|e| DriverError::Other(format!("spawn cloud-hypervisor: {e}")))?;

        let pid = child.id().unwrap_or(0);
        fs::write(dir.join("ch.pid"), pid.to_string()).await?;
        std::mem::forget(child);
        info!(%id, %pid, disk = %disk.display(), "cloud-hypervisor started");

        Ok(DriverResult {
            accepted: true,
            instance_id: id.clone(),
            driver: "cloud_hypervisor".into(),
            action: "boot".into(),
            pid: Some(pid),
            socket: None,
            tap_device: Some(tap),
            note: format!("running; {note}"),
        })
    }

    async fn stop(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        let dir = self.instance_dir(instance_id);
        if let Ok(pid_s) = fs::read_to_string(dir.join("ch.pid")).await {
            if let Ok(pid) = pid_s.trim().parse::<u32>() {
                let _ = Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status()
                    .await;
            }
            let _ = fs::remove_file(dir.join("ch.pid")).await;
        }
        if let Ok(s) = fs::read_to_string(dir.join("network.json")).await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                if let (Some(pip), Some(ps), Some(pe)) = (
                    v.get("nat_public_ip").and_then(|x| x.as_str()),
                    v.get("nat_port_start").and_then(|x| x.as_u64()),
                    v.get("nat_port_end").and_then(|x| x.as_u64()),
                ) {
                    clear_dnat(pip, &guest_ip_for(instance_id), ps as u16, pe as u16).await;
                }
            }
        }
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "cloud_hypervisor".into(),
            action: "stop".into(),
            pid: None,
            socket: None,
            tap_device: None,
            note: "stopped".into(),
        })
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
            driver: "cloud_hypervisor".into(),
            action: "destroy".into(),
            pid: None,
            socket: None,
            tap_device: None,
            note: "destroyed".into(),
        })
    }
}

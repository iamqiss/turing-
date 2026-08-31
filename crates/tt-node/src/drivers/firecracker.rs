//! Firecracker microVM driver + TAP + DNAT + deploy bake.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::json;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use super::{Driver, DriverError, DriverRequest, DriverResult, ResourceSpec};
use crate::deploy::{bake_rootfs, stage_git};
use crate::guest_net::GuestNetConfig;
use crate::bridge::{attach_tap, ensure_bridge};
use crate::cidata::build_cidata_iso;
use crate::netfilter::{apply_dnat, clear_dnat, guest_ip_for};

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

    fn tap_name(req: &DriverRequest) -> String {
        req.tap_device.clone().unwrap_or_else(|| {
            let hex = req.instance_id.replace('-', "");
            format!("tt{}", &hex[..hex.len().min(8)])
        })
    }

    fn guest_mac(instance_id: &str) -> String {
        let h: String = instance_id
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        let pad = format!("{h:0<6}");
        format!(
            "AA:FC:00:{}{}:{}{}:{}{}",
            pad.chars().nth(0).unwrap_or('0'),
            pad.chars().nth(1).unwrap_or('0'),
            pad.chars().nth(2).unwrap_or('0'),
            pad.chars().nth(3).unwrap_or('0'),
            pad.chars().nth(4).unwrap_or('0'),
            pad.chars().nth(5).unwrap_or('0'),
        )
    }

    async fn ensure_tap(&self, tap: &str) -> Result<(), DriverError> {
        let exists = Command::new("ip")
            .args(["link", "show", "dev", tap])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if exists {
            return Ok(());
        }
        match Command::new("ip")
            .args(["tuntap", "add", "dev", tap, "mode", "tap"])
            .output()
            .await
        {
            Ok(o) if o.status.success() => {
                let _ = Command::new("ip")
                    .args(["link", "set", "dev", tap, "up"])
                    .status()
                    .await;
                info!(%tap, "TAP created");
            }
            Ok(o) => {
                warn!(
                    %tap,
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "TAP create failed"
                );
            }
            Err(e) => warn!(%tap, error = %e, "ip unavailable"),
        }
        Ok(())
    }

    async fn delete_tap(&self, tap: &str) {
        let _ = Command::new("ip")
            .args(["link", "del", "dev", tap])
            .status()
            .await;
    }

    async fn write_config(
        &self,
        dir: &Path,
        resources: &ResourceSpec,
        rootfs: &Path,
        tap: &str,
        mac: &str,
        cidata: Option<&Path>,
    ) -> Result<PathBuf, DriverError> {
        let config_path = dir.join("config.json");
        let mut drives = vec![json!({
            "drive_id": "rootfs",
            "path_on_host": rootfs.to_string_lossy(),
            "is_root_device": true,
            "is_read_only": false
        })];
        if let Some(iso) = cidata {
            drives.push(json!({
                "drive_id": "cidata",
                "path_on_host": iso.to_string_lossy(),
                "is_root_device": false,
                "is_read_only": true
            }));
        }
        let cfg = json!({
            "boot-source": {
                "kernel_image_path": self.kernel_path.to_string_lossy(),
                "boot_args": "console=ttyS0 reboot=k panic=1 pci=off"
            },
            "drives": drives,
            "machine-config": {
                "vcpu_count": resources.vcpu.max(1),
                "mem_size_mib": resources.memory_mb.max(128),
                "smt": false
            },
            "network-interfaces": [{
                "iface_id": "eth0",
                "guest_mac": mac,
                "host_dev_name": tap
            }]
        });
        fs::write(&config_path, serde_json::to_vec_pretty(&cfg).unwrap()).await?;
        Ok(config_path)
    }

    fn binary_available(&self) -> bool {
        let bin = &self.firecracker_bin;
        if bin.is_absolute() {
            return bin.exists();
        }
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).exists()))
            .unwrap_or(false)
    }
}

#[async_trait]
impl Driver for FirecrackerDriver {
    fn name(&self) -> &'static str {
        "firecracker"
    }

    async fn boot(
        &self,
        req: &DriverRequest,
        resources: &ResourceSpec,
    ) -> Result<DriverResult, DriverError> {
        let instance_id = &req.instance_id;
        let dir = self.instance_dir(instance_id);
        fs::create_dir_all(&dir).await?;

        let tap = Self::tap_name(req);
        let mac = Self::guest_mac(instance_id);
        let guest_ip = guest_ip_for(instance_id);
        self.ensure_tap(&tap).await?;
        ensure_bridge().await;
        attach_tap(&tap).await;

        let mut deploy_note = String::new();
        if let Some(url) = &req.git_clone_url {
            let outcome = stage_git(&dir, url, req.git_sha.as_deref(), req.deploy_token.as_deref()).await;
            deploy_note = format!("deploy={}", outcome.status);
        }

        let meta = json!({
            "tap": tap,
            "guest_mac": mac,
            "guest_ip": guest_ip,
            "nat_public_ip": req.nat_public_ip,
            "nat_port_start": req.nat_port_start,
            "nat_port_end": req.nat_port_end,
            "git_sha": req.git_sha,
            "git_clone_url": req.git_clone_url,
        });
        fs::write(dir.join("network.json"), serde_json::to_vec_pretty(&meta).unwrap()).await?;

        let gnet_early = GuestNetConfig::from_instance(instance_id, &guest_ip);
        let _ = gnet_early.write_cloud_init(&dir).await;

        if !self.binary_available() {
            warn!("firecracker binary missing — dry-run");
            fs::write(
                dir.join("status.json"),
                serde_json::to_vec_pretty(&json!({
                    "status": "dry_run_running",
                    "driver": "firecracker",
                    "tap": tap,
                    "guest_ip": guest_ip,
                    "vcpu": resources.vcpu,
                    "memory_mb": resources.memory_mb,
                }))
                .unwrap(),
            )
            .await?;
            return Ok(DriverResult {
                accepted: true,
                instance_id: instance_id.clone(),
                driver: "firecracker".into(),
                action: "boot".into(),
                pid: None,
                socket: None,
                tap_device: Some(tap),
                note: format!("dry-run; {deploy_note}"),
            });
        }

        if !self.kernel_path.exists() {
            return Err(DriverError::MissingAsset(format!(
                "kernel missing: {}",
                self.kernel_path.display()
            )));
        }

        let rootfs_src = self.resolve_rootfs(req.image_slug.as_deref());
        let rootfs_dst = dir.join("rootfs.ext4");
        if !rootfs_src.exists() {
            return Err(DriverError::MissingAsset(format!(
                "rootfs missing: {}",
                rootfs_src.display()
            )));
        }
        if !rootfs_dst.exists() {
            fs::copy(&rootfs_src, &rootfs_dst).await?;
        }
        let src_app = dir.join("src");
        let _ = bake_rootfs(
            &rootfs_src,
            &rootfs_dst,
            if src_app.exists() {
                Some(src_app.as_path())
            } else {
                None
            },
            Some(&gnet_early),
        )
        .await;

        if let (Some(pub_ip), Some(ps), Some(pe)) =
            (&req.nat_public_ip, req.nat_port_start, req.nat_port_end)
        {
            let _ = apply_dnat(pub_ip, &guest_ip, ps, pe).await;
        }

        let cidata = build_cidata_iso(&dir).await;
        let socket = dir.join("api.sock");
        let _ = fs::remove_file(&socket).await;
        let config = self
            .write_config(&dir, resources, &rootfs_dst, &tap, &mac, cidata.as_deref())
            .await?;

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
        fs::write(dir.join("firecracker.pid"), pid.to_string()).await?;
        std::mem::forget(child);

        info!(%instance_id, %pid, %tap, %guest_ip, "firecracker booted");
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.clone(),
            driver: "firecracker".into(),
            action: "boot".into(),
            pid: Some(pid),
            socket: Some(socket.to_string_lossy().into()),
            tap_device: Some(tap),
            note: format!("firecracker+TAP+DNAT; {deploy_note}"),
        })
    }

    async fn stop(&self, instance_id: &str) -> Result<DriverResult, DriverError> {
        let dir = self.instance_dir(instance_id);
        let net: Option<serde_json::Value> = fs::read_to_string(dir.join("network.json"))
            .await
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        let tap = net
            .as_ref()
            .and_then(|v| v.get("tap").and_then(|t| t.as_str().map(str::to_string)));

        if let Ok(pid_s) = fs::read_to_string(dir.join("firecracker.pid")).await {
            if let Ok(pid) = pid_s.trim().parse::<u32>() {
                let _ = Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status()
                    .await;
            }
            let _ = fs::remove_file(dir.join("firecracker.pid")).await;
        }
        if let Some(ref t) = tap {
            self.delete_tap(t).await;
        }
        if let Some(v) = net {
            if let (Some(pip), Some(ps), Some(pe)) = (
                v.get("nat_public_ip").and_then(|x| x.as_str()),
                v.get("nat_port_start").and_then(|x| x.as_u64()),
                v.get("nat_port_end").and_then(|x| x.as_u64()),
            ) {
                clear_dnat(pip, &guest_ip_for(instance_id), ps as u16, pe as u16).await;
            }
        }
        let _ = fs::remove_file(dir.join("status.json")).await;
        Ok(DriverResult {
            accepted: true,
            instance_id: instance_id.into(),
            driver: "firecracker".into(),
            action: "stop".into(),
            pid: None,
            socket: None,
            tap_device: tap,
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
            driver: "firecracker".into(),
            action: "destroy".into(),
            pid: None,
            socket: None,
            tap_device: None,
            note: "destroyed".into(),
        })
    }
}

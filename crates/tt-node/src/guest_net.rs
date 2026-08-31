//! Guest network identity + cloud-init style config.
//!
//! Writes:
//!   instance/cloud-init/user-data
//!   instance/cloud-init/meta-data
//!   instance/cloud-init/network-config  (v2 netplan-ish for cloud-init)
//!
//! When rootfs is loop-mounted during bake, injects:
//!   /etc/tt-net.env
//!   /etc/netplan/99-tt.yaml (if netplan dir exists)
//!   /etc/network/interfaces.d/tt (debian-style)

use std::path::Path;

use serde_json::json;
use tokio::fs;
use tracing::{info, warn};

pub struct GuestNetConfig {
    pub instance_id: String,
    pub guest_ip: String,
    /// Point-to-point / host gateway on TAP side
    pub gateway: String,
    pub prefix: u8,
    pub hostname: String,
}

impl GuestNetConfig {
    pub fn from_instance(instance_id: &str, guest_ip: &str) -> Self {
        // Gateway: same /24 as guest, .1
        let gateway = {
            let mut parts: Vec<&str> = guest_ip.split('.').collect();
            if parts.len() == 4 {
                parts[3] = "1";
                parts.join(".")
            } else {
                "10.200.0.1".into()
            }
        };
        let short = instance_id.chars().take(8).collect::<String>();
        Self {
            instance_id: instance_id.into(),
            guest_ip: guest_ip.into(),
            gateway,
            prefix: 24,
            hostname: format!("tt-{short}"),
        }
    }

    pub async fn write_cloud_init(&self, instance_dir: &Path) -> std::io::Result<()> {
        let dir = instance_dir.join("cloud-init");
        fs::create_dir_all(&dir).await?;

        let meta = format!(
            "instance-id: {}\nlocal-hostname: {}\n",
            self.instance_id, self.hostname
        );
        fs::write(dir.join("meta-data"), meta).await?;

        let user_data = format!(
            r#"#cloud-config
hostname: {host}
fqdn: {host}.tt.local
manage_etc_hosts: true
users:
  - name: tt
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
write_files:
  - path: /etc/tt-net.env
    content: |
      TT_GUEST_IP={ip}
      TT_GATEWAY={gw}
      TT_PREFIX={prefix}
      TT_INSTANCE_ID={id}
runcmd:
  - |
    if command -v ip >/dev/null; then
      ip addr add {ip}/{prefix} dev eth0 2>/dev/null || true
      ip link set eth0 up 2>/dev/null || true
      ip route add default via {gw} 2>/dev/null || true
    fi
"#,
            host = self.hostname,
            ip = self.guest_ip,
            gw = self.gateway,
            prefix = self.prefix,
            id = self.instance_id,
        );
        fs::write(dir.join("user-data"), user_data).await?;

        let network_config = format!(
            r#"version: 2
ethernets:
  eth0:
    addresses:
      - {ip}/{prefix}
    gateway4: {gw}
    nameservers:
      addresses: [1.1.1.1, 8.8.8.8]
"#,
            ip = self.guest_ip,
            prefix = self.prefix,
            gw = self.gateway,
        );
        fs::write(dir.join("network-config"), network_config).await?;

        // Sidecar JSON for operators
        fs::write(
            instance_dir.join("guest_net.json"),
            serde_json::to_vec_pretty(&json!({
                "guest_ip": self.guest_ip,
                "gateway": self.gateway,
                "prefix": self.prefix,
                "hostname": self.hostname,
            }))
            .unwrap(),
        )
        .await?;

        info!(
            guest_ip = %self.guest_ip,
            hostname = %self.hostname,
            "cloud-init guest net written"
        );
        Ok(())
    }

    /// Inject static net config into a mounted rootfs tree.
    pub async fn inject_into_rootfs(&self, mnt: &Path) {
        let env = format!(
            "TT_GUEST_IP={}\nTT_GATEWAY={}\nTT_PREFIX={}\nTT_INSTANCE_ID={}\n",
            self.guest_ip, self.gateway, self.prefix, self.instance_id
        );
        if let Some(parent) = mnt.join("etc").parent() {
            let _ = parent;
        }
        let _ = fs::create_dir_all(mnt.join("etc")).await;
        if let Err(e) = fs::write(mnt.join("etc/tt-net.env"), &env).await {
            warn!(error = %e, "write tt-net.env failed");
        }

        let netplan_dir = mnt.join("etc/netplan");
        if netplan_dir.exists() || fs::create_dir_all(&netplan_dir).await.is_ok() {
            let yaml = format!(
                r#"network:
  version: 2
  ethernets:
    eth0:
      addresses: [{ip}/{prefix}]
      gateway4: {gw}
      nameservers:
        addresses: [1.1.1.1, 8.8.8.8]
"#,
                ip = self.guest_ip,
                prefix = self.prefix,
                gw = self.gateway,
            );
            let _ = fs::write(netplan_dir.join("99-tt.yaml"), yaml).await;
        }

        let ifaces = mnt.join("etc/network/interfaces.d");
        if fs::create_dir_all(&ifaces).await.is_ok() {
            let stanza = format!(
                "auto eth0\niface eth0 inet static\n    address {}\n    netmask 255.255.255.0\n    gateway {}\n",
                self.guest_ip, self.gateway
            );
            let _ = fs::write(ifaces.join("tt"), stanza).await;
        }
    }
}

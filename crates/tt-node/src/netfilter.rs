//! Host NAT / DNAT helpers (iptables).
//!
//! Maps public_ip:port_start-port_end → guest_ip:same ports (TCP+UDP).
//! Soft-fail without root or iptables.

use tokio::process::Command;
use tracing::{info, warn};

pub async fn apply_dnat(
    public_ip: &str,
    guest_ip: &str,
    port_start: u16,
    port_end: u16,
) -> bool {
    let range = format!("{port_start}:{port_end}");
    // iptables -t nat -A PREROUTING -d PUBLIC -p tcp --dport RANGE -j DNAT --to-destination GUEST
    let tcp = iptables(&[
        "-t",
        "nat",
        "-A",
        "PREROUTING",
        "-d",
        public_ip,
        "-p",
        "tcp",
        "--dport",
        &range,
        "-j",
        "DNAT",
        "--to-destination",
        &format!("{guest_ip}:{range}"),
    ])
    .await;
    let udp = iptables(&[
        "-t",
        "nat",
        "-A",
        "PREROUTING",
        "-d",
        public_ip,
        "-p",
        "udp",
        "--dport",
        &range,
        "-j",
        "DNAT",
        "--to-destination",
        &format!("{guest_ip}:{range}"),
    ])
    .await;
    // MASQUERADE return path
    let masq = iptables(&[
        "-t",
        "nat",
        "-A",
        "POSTROUTING",
        "-s",
        guest_ip,
        "-j",
        "MASQUERADE",
    ])
    .await;
    if tcp && udp {
        info!(%public_ip, %guest_ip, %range, "DNAT applied");
        true
    } else {
        warn!(%public_ip, %guest_ip, "DNAT incomplete (need root + iptables)");
        let _ = masq;
        false
    }
}

pub async fn clear_dnat(
    public_ip: &str,
    guest_ip: &str,
    port_start: u16,
    port_end: u16,
) {
    let range = format!("{port_start}:{port_end}");
    let _ = iptables(&[
        "-t",
        "nat",
        "-D",
        "PREROUTING",
        "-d",
        public_ip,
        "-p",
        "tcp",
        "--dport",
        &range,
        "-j",
        "DNAT",
        "--to-destination",
        &format!("{guest_ip}:{range}"),
    ])
    .await;
    let _ = iptables(&[
        "-t",
        "nat",
        "-D",
        "PREROUTING",
        "-d",
        public_ip,
        "-p",
        "udp",
        "--dport",
        &range,
        "-j",
        "DNAT",
        "--to-destination",
        &format!("{guest_ip}:{range}"),
    ])
    .await;
    let _ = iptables(&[
        "-t",
        "nat",
        "-D",
        "POSTROUTING",
        "-s",
        guest_ip,
        "-j",
        "MASQUERADE",
    ])
    .await;
}

async fn iptables(args: &[&str]) -> bool {
    match Command::new("iptables").args(args).output().await {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            warn!(
                stderr = %String::from_utf8_lossy(&o.stderr),
                "iptables failed"
            );
            false
        }
        Err(e) => {
            warn!(error = %e, "iptables not available");
            false
        }
    }
}

/// Deterministic guest IP in 10.200.0.0/16 from instance id hex.
pub fn guest_ip_for(instance_id: &str) -> String {
    let h = instance_id.replace('-', "");
    let bytes: Vec<u8> = (0..h.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).ok())
        .collect();
    let a = bytes.first().copied().unwrap_or(1).max(1);
    let b = bytes.get(1).copied().unwrap_or(1).max(1);
    format!("10.200.{a}.{b}")
}

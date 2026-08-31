//! Host bridge networking for TAPs.
//!
//! Env:
//!   TT_BRIDGE=br-tt          (default)
//!   TT_BRIDGE_ADDR=10.200.0.1/16  (optional, set on bridge once)

use tokio::process::Command;
use tracing::{info, warn};

pub fn bridge_name() -> String {
    std::env::var("TT_BRIDGE").unwrap_or_else(|_| "br-tt".into())
}

pub async fn ensure_bridge() {
    let br = bridge_name();
    let exists = Command::new("ip")
        .args(["link", "show", "dev", &br])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !exists {
        match Command::new("ip")
            .args(["link", "add", "name", &br, "type", "bridge"])
            .output()
            .await
        {
            Ok(o) if o.status.success() => {
                let _ = Command::new("ip")
                    .args(["link", "set", "dev", &br, "up"])
                    .status()
                    .await;
                if let Ok(addr) = std::env::var("TT_BRIDGE_ADDR") {
                    let _ = Command::new("ip")
                        .args(["addr", "add", &addr, "dev", &br])
                        .status()
                        .await;
                } else {
                    let _ = Command::new("ip")
                        .args(["addr", "add", "10.200.0.1/16", "dev", &br])
                        .status()
                        .await;
                }
                info!(%br, "bridge created");
            }
            Ok(o) => warn!(
                stderr = %String::from_utf8_lossy(&o.stderr),
                "bridge create failed"
            ),
            Err(e) => warn!(error = %e, "ip unavailable for bridge"),
        }
    }
}

pub async fn attach_tap(tap: &str) {
    let br = bridge_name();
    ensure_bridge().await;
    let _ = Command::new("ip")
        .args(["link", "set", "dev", tap, "master", &br])
        .status()
        .await;
    let _ = Command::new("ip")
        .args(["link", "set", "dev", tap, "up"])
        .status()
        .await;
    info!(%tap, %br, "TAP attached to bridge");
}

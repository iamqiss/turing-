//! tt-kms binary — run on TT bare metal, private NIC only.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};
use tt_kms::{
    generate_master_key, router, seal_master_key, unseal_master_key, AppState, KmsEngine,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tt_kms=info".into()),
        )
        .init();

    let bind = std::env::var("TT_KMS_BIND").unwrap_or_else(|_| "127.0.0.1:9797".into());
    let key_id = std::env::var("TT_KMS_KEY_ID").unwrap_or_else(|_| "scarlet-v1".into());
    let sealed_path = PathBuf::from(
        std::env::var("TT_KMS_SEALED_PATH").unwrap_or_else(|_| "data/kms/master.sealed".into()),
    );
    let passphrase = std::env::var("TT_KMS_UNSEAL").unwrap_or_default();
    let auth_secret = std::env::var("TT_KMS_AUTH_SECRET").unwrap_or_default();

    let engine = KmsEngine::new_locked();

    if passphrase.is_empty() {
        warn!("TT_KMS_UNSEAL empty — starting LOCKED (set passphrase to unseal)");
    } else if sealed_path.exists() {
        let blob = tokio::fs::read(&sealed_path).await?;
        let master = unseal_master_key(&blob, &passphrase)?;
        engine.unlock(&key_id, master).await?;
        info!(%key_id, path = %sealed_path.display(), "unsealed master key");
    } else {
        // First boot: generate + seal
        if let Some(parent) = sealed_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let master = generate_master_key();
        let sealed = seal_master_key(&master, &passphrase)?;
        tokio::fs::write(&sealed_path, &sealed).await?;
        engine.unlock(&key_id, master).await?;
        info!(%key_id, path = %sealed_path.display(), "generated + sealed new master key");
    }

    let state = AppState {
        engine: engine.clone(),
        auth_secret,
    };
    let app = router(state);
    let addr: SocketAddr = bind.parse()?;
    info!(%addr, "tt-kms listening (bind private NIC only on metal)");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

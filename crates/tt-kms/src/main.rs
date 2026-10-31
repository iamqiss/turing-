//! tt-kms binary — run on TT bare metal, private NIC only.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};
use tt_kms::{
    generate_master_key, is_legacy_seal, router, seal_master_key, unseal_master_key,
    write_sealed_file, AppState, KmsEngine,
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
    let insecure_lab = std::env::var("TT_KMS_INSECURE_LAB").is_ok_and(|v| v == "1");

    // Fail closed: wrap/unwrap hand out DEKs, so never serve them unauthenticated
    // by accident.
    if auth_secret.is_empty() {
        if !insecure_lab {
            anyhow::bail!(
                "TT_KMS_AUTH_SECRET is empty: refusing to serve unauthenticated wrap/unwrap \
                 (set TT_KMS_INSECURE_LAB=1 for a local lab only)"
            );
        }
        warn!("TT_KMS_INSECURE_LAB=1: wrap/unwrap are UNAUTHENTICATED — lab use only");
    } else if auth_secret.len() < 32 {
        anyhow::bail!("TT_KMS_AUTH_SECRET must be at least 32 bytes");
    }

    let engine = KmsEngine::new_locked();

    if passphrase.is_empty() {
        warn!("TT_KMS_UNSEAL empty — starting LOCKED (set passphrase to unseal)");
    } else if sealed_path.exists() {
        let blob = tokio::fs::read(&sealed_path).await?;
        let master = unseal_master_key(&blob, &passphrase)?;
        if is_legacy_seal(&blob) {
            // v1 used one SHA-256 of the passphrase: upgrade to Argon2id in place.
            let upgraded = seal_master_key(&master, &passphrase)?;
            // never replace the only copy of the key with something unverified
            if unseal_master_key(&upgraded, &passphrase)? != master {
                anyhow::bail!("sealed-key upgrade self-check failed; file left unchanged");
            }
            let mut backup = sealed_path.as_os_str().to_owned();
            backup.push(".v1.bak");
            let backup = PathBuf::from(backup);
            write_sealed_file(&backup, &blob)?;
            write_sealed_file(&sealed_path, &upgraded)?;
            warn!(
                path = %sealed_path.display(),
                backup = %backup.display(),
                "upgraded sealed master key to Argon2id (v2); the .v1.bak copy is brute-forceable — \
                 delete it once a restart has been confirmed"
            );
        }
        engine.unlock(&key_id, master).await?;
        info!(%key_id, path = %sealed_path.display(), "unsealed master key");
    } else {
        // First boot: generate + seal
        if let Some(parent) = sealed_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let master = generate_master_key();
        let sealed = seal_master_key(&master, &passphrase)?;
        write_sealed_file(&sealed_path, &sealed)?;
        engine.unlock(&key_id, master).await?;
        info!(%key_id, path = %sealed_path.display(), "generated + sealed new master key");
    }

    let state = AppState::new(engine.clone(), auth_secret);
    let app = router(state);
    let addr: SocketAddr = bind.parse()?;
    info!(%addr, "tt-kms listening (bind private NIC only on metal)");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

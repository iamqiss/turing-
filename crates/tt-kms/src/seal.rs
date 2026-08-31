//! Operator seal/unseal for master KEK material at rest on TT disk.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::{KmsError, Result};

/// Derive a 32-byte root key from operator secret (lab: SHA-256; prod: Argon2 later).
pub fn derive_root(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"tt-kms-v1");
    h.update(salt);
    h.update(passphrase.as_bytes());
    let out = h.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

pub fn generate_master_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Seal master key for disk: salt(16) || nonce(12) || ciphertext
pub fn seal_master_key(master: &[u8; 32], passphrase: &str) -> Result<Vec<u8>> {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let root = derive_root(passphrase, &salt);
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&root)
        .map_err(|e| KmsError::Crypto(e.to_string()))?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), master.as_slice())
        .map_err(|_| KmsError::Crypto("seal master".into()))?;
    let mut out = Vec::with_capacity(16 + 12 + ct.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn unseal_master_key(blob: &[u8], passphrase: &str) -> Result<[u8; 32]> {
    if blob.len() < 16 + 12 + 16 {
        return Err(KmsError::Invalid("sealed blob too short".into()));
    }
    let (salt, rest) = blob.split_at(16);
    let (nonce, ct) = rest.split_at(12);
    let root = derive_root(passphrase, salt);
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&root)
        .map_err(|e| KmsError::Crypto(e.to_string()))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| KmsError::Crypto("unseal failed — bad passphrase?".into()))?;
    if plain.len() != 32 {
        return Err(KmsError::Crypto("bad master length".into()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&plain);
    Ok(key)
}

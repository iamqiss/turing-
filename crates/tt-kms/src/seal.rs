//! Operator seal/unseal for master KEK material at rest on TT disk.
//!
//! Two on-disk formats:
//!
//! - **v1 (legacy, read-only):** `salt(16) || nonce(12) || AES-256-GCM(master)`,
//!   root key = `SHA-256("tt-kms-v1" || salt || passphrase)`. A single fast hash:
//!   anyone who copies the file can try passphrases at billions per second.
//!   Still readable so existing deployments boot, then upgraded in place.
//! - **v2:** `MAGIC(8) || m_kib(4) || t(4) || p(4) || salt(16) || nonce(12) || ct`
//!   (integers big-endian), root key = Argon2id(passphrase, salt, m, t, p).
//!   The 36-byte header is bound as AES-GCM associated data, so the parameters
//!   cannot be altered without detection.

use std::io::Write;
use std::path::Path;

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::{KmsError, Result};

/// File magic for the v2 format.
pub const SEAL_V2_MAGIC: &[u8; 8] = b"TTKMSv2\0";
/// Default Argon2id cost for new seals: 64 MiB, 3 passes, 1 lane.
/// This only runs at tt-kms boot, so it can be much heavier than a login hash.
pub const ARGON_M_KIB: u32 = 65_536;
pub const ARGON_T: u32 = 3;
pub const ARGON_P: u32 = 1;

// Upper bounds when *reading* a file, so a corrupted/hostile header cannot make
// boot allocate unbounded memory or spin for hours.
const MAX_M_KIB: u32 = 1_048_576; // 1 GiB
const MAX_T: u32 = 16;
const MAX_P: u32 = 16;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = 8 + 4 + 4 + 4 + SALT_LEN;
const GCM_TAG_LEN: usize = 16;
const MASTER_LEN: usize = 32;

/// v1 root key derivation. Kept only to read legacy files; never use for new data.
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

fn derive_root_v2(passphrase: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<[u8; 32]> {
    let params = Params::new(m, t, p, Some(32))
        .map_err(|e| KmsError::Crypto(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| KmsError::Crypto(format!("argon2: {e}")))?;
    Ok(out)
}

pub fn generate_master_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// True when `blob` is the legacy SHA-256 format and should be upgraded.
pub fn is_legacy_seal(blob: &[u8]) -> bool {
    !blob.starts_with(SEAL_V2_MAGIC)
}

/// Seal the master key for disk with the default Argon2id cost (v2 format).
pub fn seal_master_key(master: &[u8; 32], passphrase: &str) -> Result<Vec<u8>> {
    seal_master_key_with(master, passphrase, ARGON_M_KIB, ARGON_T, ARGON_P)
}

/// Seal with explicit Argon2id cost. Production code uses [`seal_master_key`];
/// tests use small parameters to stay fast.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn seal_master_key_with(
    master: &[u8; 32],
    passphrase: &str,
    m: u32,
    t: u32,
    p: u32,
) -> Result<Vec<u8>> {
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(SEAL_V2_MAGIC);
    header.extend_from_slice(&m.to_be_bytes());
    header.extend_from_slice(&t.to_be_bytes());
    header.extend_from_slice(&p.to_be_bytes());
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    header.extend_from_slice(&salt);

    let root = derive_root_v2(passphrase, &salt, m, t, p)?;
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&root)
        .map_err(|e| KmsError::Crypto(e.to_string()))?;
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: master.as_slice(),
                aad: &header,
            },
        )
        .map_err(|_| KmsError::Crypto("seal master".into()))?;

    let mut out = header;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a sealed master key. Accepts v2 and legacy v1 blobs.
pub fn unseal_master_key(blob: &[u8], passphrase: &str) -> Result<[u8; 32]> {
    if blob.starts_with(SEAL_V2_MAGIC) {
        unseal_v2(blob, passphrase)
    } else {
        unseal_v1(blob, passphrase)
    }
}

fn unseal_v2(blob: &[u8], passphrase: &str) -> Result<[u8; 32]> {
    if blob.len() != HEADER_LEN + NONCE_LEN + MASTER_LEN + GCM_TAG_LEN {
        return Err(KmsError::Invalid("sealed blob has wrong length".into()));
    }
    let (header, rest) = blob.split_at(HEADER_LEN);
    let (nonce, ct) = rest.split_at(NONCE_LEN);
    let be = |i: usize| u32::from_be_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let (m, t, p) = (be(8), be(12), be(16));
    if m == 0 || t == 0 || p == 0 || m > MAX_M_KIB || t > MAX_T || p > MAX_P {
        return Err(KmsError::Invalid("sealed blob has unreasonable KDF parameters".into()));
    }
    let salt = &header[20..20 + SALT_LEN];

    let root = derive_root_v2(passphrase, salt, m, t, p)?;
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&root)
        .map_err(|e| KmsError::Crypto(e.to_string()))?;
    let plain = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: header,
            },
        )
        .map_err(|_| KmsError::Crypto("unseal failed — bad passphrase or corrupted file".into()))?;
    to_master(&plain)
}

fn unseal_v1(blob: &[u8], passphrase: &str) -> Result<[u8; 32]> {
    if blob.len() < SALT_LEN + NONCE_LEN + GCM_TAG_LEN {
        return Err(KmsError::Invalid("sealed blob too short".into()));
    }
    let (salt, rest) = blob.split_at(SALT_LEN);
    let (nonce, ct) = rest.split_at(NONCE_LEN);
    let root = derive_root(passphrase, salt);
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&root)
        .map_err(|e| KmsError::Crypto(e.to_string()))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| KmsError::Crypto("unseal failed — bad passphrase or corrupted file".into()))?;
    to_master(&plain)
}

fn to_master(plain: &[u8]) -> Result<[u8; 32]> {
    if plain.len() != MASTER_LEN {
        return Err(KmsError::Crypto("bad master length".into()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(plain);
    Ok(key)
}

/// Write `bytes` to `path` atomically (temp file + fsync + rename) with mode
/// 0600 on unix, so a crash cannot leave a truncated sealed key.
pub fn write_sealed_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp_name);

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Small cost so tests are fast in debug builds.
    const M: u32 = 64;
    const T: u32 = 1;
    const P: u32 = 1;

    fn v1_blob(master: &[u8; 32], passphrase: &str) -> Vec<u8> {
        // What the old seal_master_key produced.
        let salt = [5u8; 16];
        let nonce = [6u8; 12];
        let root = derive_root(passphrase, &salt);
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&root).unwrap();
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), master.as_slice())
            .unwrap();
        let mut out = salt.to_vec();
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    #[test]
    fn seal_unseal_roundtrip() {
        let master = generate_master_key();
        let sealed = seal_master_key_with(&master, "correct-horse", M, T, P).unwrap();
        assert!(sealed.starts_with(SEAL_V2_MAGIC));
        assert!(!is_legacy_seal(&sealed));
        assert_eq!(unseal_master_key(&sealed, "correct-horse").unwrap(), master);
    }

    #[test]
    fn bad_passphrase_fails() {
        let master = generate_master_key();
        let sealed = seal_master_key_with(&master, "right", M, T, P).unwrap();
        assert!(unseal_master_key(&sealed, "wrong").is_err());
    }

    #[test]
    fn legacy_v1_files_still_unseal_and_are_flagged() {
        let master = [0x42u8; 32];
        let old = v1_blob(&master, "old-pass");
        assert!(is_legacy_seal(&old));
        assert_eq!(unseal_master_key(&old, "old-pass").unwrap(), master);
        assert!(unseal_master_key(&old, "nope").is_err());
    }

    #[test]
    fn upgrade_path_preserves_the_master_key() {
        let master = [0x11u8; 32];
        let old = v1_blob(&master, "pw");
        let opened = unseal_master_key(&old, "pw").unwrap();
        let upgraded = seal_master_key_with(&opened, "pw", M, T, P).unwrap();
        assert!(!is_legacy_seal(&upgraded));
        assert_eq!(unseal_master_key(&upgraded, "pw").unwrap(), master);
    }

    #[test]
    fn two_seals_of_the_same_key_differ() {
        let master = [1u8; 32];
        let a = seal_master_key_with(&master, "pw", M, T, P).unwrap();
        let b = seal_master_key_with(&master, "pw", M, T, P).unwrap();
        assert_ne!(a, b); // fresh salt + nonce
    }

    #[test]
    fn tampering_with_kdf_params_salt_or_ciphertext_is_detected() {
        let master = [9u8; 32];
        let sealed = seal_master_key_with(&master, "pw", M, T, P).unwrap();
        // flip one bit in: m, t, p, salt, nonce, ciphertext, tag
        for idx in [11, 15, 19, 25, HEADER_LEN + 1, HEADER_LEN + NONCE_LEN + 1, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[idx] ^= 1;
            assert!(unseal_master_key(&bad, "pw").is_err(), "byte {idx} not authenticated");
        }
    }

    #[test]
    fn rejects_truncated_and_extended_v2_blobs() {
        let sealed = seal_master_key_with(&[3u8; 32], "pw", M, T, P).unwrap();
        assert!(unseal_master_key(&sealed[..sealed.len() - 1], "pw").is_err());
        let mut longer = sealed.clone();
        longer.push(0);
        assert!(unseal_master_key(&longer, "pw").is_err());
        assert!(unseal_master_key(&[], "pw").is_err());
        assert!(unseal_master_key(SEAL_V2_MAGIC, "pw").is_err());
    }

    #[test]
    fn hostile_kdf_parameters_are_refused_before_hashing() {
        let sealed = seal_master_key_with(&[3u8; 32], "pw", M, T, P).unwrap();
        for (off, val) in [(8usize, u32::MAX), (12, u32::MAX), (16, u32::MAX), (8, 0), (12, 0), (16, 0)] {
            let mut bad = sealed.clone();
            bad[off..off + 4].copy_from_slice(&val.to_be_bytes());
            let err = unseal_master_key(&bad, "pw").unwrap_err().to_string();
            assert!(err.contains("unreasonable"), "off {off} val {val}: {err}");
        }
    }

    #[test]
    fn default_cost_is_at_least_owasp_minimum() {
        assert!(ARGON_M_KIB >= 19_456);
        assert!(ARGON_T >= 2);
        assert!(ARGON_P >= 1);
        assert!(ARGON_M_KIB <= MAX_M_KIB && ARGON_T <= MAX_T && ARGON_P <= MAX_P);
    }

    #[test]
    fn write_sealed_file_is_atomic_and_private() {
        let dir = std::env::temp_dir().join(format!("tt-kms-seal-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("master.sealed");
        write_sealed_file(&path, b"one").unwrap();
        write_sealed_file(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(!dir.join("master.sealed.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

//! AES-256-GCM secret box for tokens at rest + HMAC request signatures.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encrypt failed")]
    Encrypt,
    #[error("decrypt failed")]
    Decrypt,
    #[error("invalid signature")]
    BadSignature,
    #[error("timestamp skew")]
    Skew,
}

#[derive(Clone)]
pub struct SecretBox {
    key: [u8; 32],
}

impl SecretBox {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// ciphertext = nonce(12) || ciphertext+tag
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&self.key).map_err(|_| CryptoError::Encrypt)?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let mut out = nonce_bytes.to_vec();
        let ct = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| CryptoError::Encrypt)?;
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if blob.len() < 12 + 16 {
            return Err(CryptoError::Decrypt);
        }
        let (nonce_bytes, ct) = blob.split_at(12);
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&self.key).map_err(|_| CryptoError::Decrypt)?;
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ct)
            .map_err(|_| CryptoError::Decrypt)
    }

    pub fn encrypt_str(&self, s: &str) -> Result<Vec<u8>, CryptoError> {
        self.encrypt(s.as_bytes())
    }

    pub fn decrypt_str(&self, blob: &[u8]) -> Result<String, CryptoError> {
        let b = self.decrypt(blob)?;
        String::from_utf8(b).map_err(|_| CryptoError::Decrypt)
    }
}

/// Sign: hex(hmac_sha256(key, "{ts}.{body}"))
pub fn sign_request(agent_token: &str, ts: i64, body: &[u8]) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(agent_token.as_bytes())
        .expect("hmac key");
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_request(
    agent_token: &str,
    ts: i64,
    body: &[u8],
    signature_hex: &str,
    max_skew_secs: i64,
) -> Result<(), CryptoError> {
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > max_skew_secs {
        return Err(CryptoError::Skew);
    }
    let expected = sign_request(agent_token, ts, body);
    if expected.len() != signature_hex.len()
        || !expected
            .as_bytes()
            .iter()
            .zip(signature_hex.as_bytes())
            .map(|(a, b)| a ^ b)
            .fold(0u8, |acc, x| acc | x)
            == 0
    {
        return Err(CryptoError::BadSignature);
    }
    Ok(())
}

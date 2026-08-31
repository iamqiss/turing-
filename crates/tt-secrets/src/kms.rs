//! KMS-backed KEK providers for Scarlet Letter envelope encryption.
//!
//! Primary path: **Turing Tarpit's own `tt-kms`** on TT bare metal
//! (Latitude.sh / OVHcloud / …). No competitor KMS required for production.
//!
//! Providers:
//! - `tt` / `tt-kms` — in-house KMS HTTP API (`tt-kms` binary)
//! - `local`         — in-process AES (single-node lab only)
//! - `http`          — generic HTTP (legacy / optional)

use async_trait::async_trait;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{Result, SecretsError};

const MAGIC: &[u8; 4] = b"TTK1";

#[async_trait]
pub trait KeyProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn key_id(&self) -> &str;
    async fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>>;
    async fn unwrap_dek(&self, ciphertext: &[u8]) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone)]
pub struct KmsConfig {
    /// tt | local | http
    pub provider: String,
    pub local_key_hex: String,
    pub key_id: String,
    /// tt-kms base URL, e.g. http://10.0.0.5:9797
    pub endpoint: String,
    /// HMAC shared secret with tt-kms (TT_KMS_AUTH_SECRET)
    pub token: String,
    pub region: String,
}

impl Default for KmsConfig {
    fn default() -> Self {
        Self {
            provider: "tt".into(),
            local_key_hex: String::new(),
            key_id: "scarlet-v1".into(),
            endpoint: "http://127.0.0.1:9797".into(),
            token: String::new(),
            region: String::new(),
        }
    }
}

pub fn build_provider(cfg: &KmsConfig) -> Result<std::sync::Arc<dyn KeyProvider>> {
    match cfg.provider.to_ascii_lowercase().as_str() {
        "tt" | "tt-kms" | "inhouse" => {
            if cfg.endpoint.is_empty() {
                return Err(SecretsError::Crypto(
                    "tt-kms provider requires kms_endpoint (e.g. http://kms-host:9797)".into(),
                ));
            }
            info!(endpoint = %cfg.endpoint, key_id = %cfg.key_id, "KMS provider: tt-kms (in-house)");
            Ok(std::sync::Arc::new(TtKmsProvider {
                endpoint: cfg.endpoint.trim_end_matches('/').to_string(),
                auth_secret: cfg.token.clone(),
                key_id: cfg.key_id.clone(),
                client: reqwest::Client::new(),
            }))
        }
        "local" | "" => {
            let key = parse_local_key(&cfg.local_key_hex)?;
            info!(key_id = %cfg.key_id, "KMS provider: local (lab only — use tt-kms on metal)");
            Ok(std::sync::Arc::new(LocalKeyProvider {
                key,
                key_id: if cfg.key_id.is_empty() {
                    "local".into()
                } else {
                    cfg.key_id.clone()
                },
            }))
        }
        "http" => {
            if cfg.endpoint.is_empty() || cfg.key_id.is_empty() {
                return Err(SecretsError::Crypto(
                    "http KMS requires endpoint + key_id".into(),
                ));
            }
            info!(endpoint = %cfg.endpoint, key_id = %cfg.key_id, "KMS provider: generic http");
            Ok(std::sync::Arc::new(HttpTransitProvider {
                endpoint: cfg.endpoint.trim_end_matches('/').to_string(),
                token: cfg.token.clone(),
                key_id: cfg.key_id.clone(),
                client: reqwest::Client::new(),
            }))
        }
        other => Err(SecretsError::Crypto(format!(
            "unknown kms provider: {other} (use tt, local, or http)"
        ))),
    }
}
fn parse_local_key(hex_or_empty: &str) -> Result<[u8; 32]> {
    let s = hex_or_empty.trim();
    if s.is_empty() {
        // Deterministic weak fallback for lab only — operators must set TT_SECRETS_KEY in prod
        let mut out = [0u8; 32];
        out[..16].copy_from_slice(b"tt-dev-only-kek!");
        out[16..].copy_from_slice(b"-secrets-key!!!");
        return Ok(out);
    }
    let bytes = hex::decode(s).map_err(|e| SecretsError::Crypto(format!("local key hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(SecretsError::Crypto(
            "local KEK must be 32 bytes (64 hex chars)".into(),
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

// ── Local ────────────────────────────────────────────────────────────


// ── Turing Tarpit in-house KMS ───────────────────────────────────────

pub struct TtKmsProvider {
    endpoint: String,
    auth_secret: String,
    key_id: String,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct TtWrapReq {
    key_id: Option<String>,
    plaintext_b64: String,
}

#[derive(Deserialize)]
struct TtWrapResp {
    ciphertext_b64: String,
    #[serde(default)]
    key_id: String,
}

#[derive(Serialize)]
struct TtUnwrapReq {
    key_id: Option<String>,
    ciphertext_b64: String,
}

#[derive(Deserialize)]
struct TtUnwrapResp {
    plaintext_b64: String,
}

fn b64_enc(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let mut n = (chunk[0] as u32) << 16;
        if chunk.len() > 1 {
            n |= (chunk[1] as u32) << 8;
        }
        if chunk.len() > 2 {
            n |= chunk[2] as u32;
        }
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn b64_dec(s: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let s = s.trim().as_bytes();
    let mut out = Vec::new();
    let mut buf = [0u8; 4];
    let mut n = 0;
    for &c in s {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        buf[n] = val(c).ok_or_else(|| SecretsError::Crypto("b64".into()))?;
        n += 1;
        if n == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
            n = 0;
        }
    }
    if n >= 2 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
        if n == 3 {
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
    }
    Ok(out)
}

fn sign_headers(secret: &str) -> Result<(String, String)> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let ts = chrono::Utc::now().timestamp().to_string();
    if secret.is_empty() {
        return Ok((ts, String::new()));
    }
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes())
        .map_err(|e| SecretsError::Crypto(e.to_string()))?;
    mac.update(ts.as_bytes());
    Ok((ts, hex::encode(mac.finalize().into_bytes())))
}

#[async_trait]
impl KeyProvider for TtKmsProvider {
    fn provider_name(&self) -> &str {
        "tt"
    }
    fn key_id(&self) -> &str {
        &self.key_id
    }

    async fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>> {
        let url = format!("{}/v1/wrap", self.endpoint);
        let body = TtWrapReq {
            key_id: Some(self.key_id.clone()),
            plaintext_b64: b64_enc(dek),
        };
        let (ts, sig) = sign_headers(&self.auth_secret)?;
        let mut req = self.client.post(&url).json(&body).header("x-tt-kms-timestamp", &ts);
        if !sig.is_empty() {
            req = req.header("x-tt-kms-signature", &sig);
        }
        let res = req
            .send()
            .await
            .map_err(|e| SecretsError::Crypto(format!("tt-kms wrap unreachable: {e}")))?;
        if !res.status().is_success() {
            let t = res.text().await.unwrap_or_default();
            return Err(SecretsError::Crypto(format!("tt-kms wrap: {t}")));
        }
        let parsed: TtWrapResp = res
            .json()
            .await
            .map_err(|e| SecretsError::Crypto(format!("tt-kms wrap json: {e}")))?;
        b64_dec(&parsed.ciphertext_b64)
    }

    async fn unwrap_dek(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let url = format!("{}/v1/unwrap", self.endpoint);
        let body = TtUnwrapReq {
            key_id: Some(self.key_id.clone()),
            ciphertext_b64: b64_enc(ciphertext),
        };
        let (ts, sig) = sign_headers(&self.auth_secret)?;
        let mut req = self.client.post(&url).json(&body).header("x-tt-kms-timestamp", &ts);
        if !sig.is_empty() {
            req = req.header("x-tt-kms-signature", &sig);
        }
        let res = req
            .send()
            .await
            .map_err(|e| SecretsError::Crypto(format!("tt-kms unwrap unreachable: {e}")))?;
        if !res.status().is_success() {
            let t = res.text().await.unwrap_or_default();
            return Err(SecretsError::Crypto(format!("tt-kms unwrap: {t}")));
        }
        let parsed: TtUnwrapResp = res
            .json()
            .await
            .map_err(|e| SecretsError::Crypto(format!("tt-kms unwrap json: {e}")))?;
        b64_dec(&parsed.plaintext_b64)
    }
}

pub struct LocalKeyProvider {
    pub key: [u8; 32],
    pub key_id: String,
}

#[async_trait]
impl KeyProvider for LocalKeyProvider {
    fn provider_name(&self) -> &str {
        "local"
    }
    fn key_id(&self) -> &str {
        &self.key_id
    }

    async fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>> {
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&self.key)
            .map_err(|e| SecretsError::Crypto(e.to_string()))?;
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), dek)
            .map_err(|_| SecretsError::Crypto("local wrap".into()))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        Ok(out)
    }

    async fn unwrap_dek(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.len() < 12 + 16 {
            return Err(SecretsError::Crypto("local wrap too short".into()));
        }
        let (n, ct) = ciphertext.split_at(12);
        let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&self.key)
            .map_err(|e| SecretsError::Crypto(e.to_string()))?;
        cipher
            .decrypt(Nonce::from_slice(n), ct)
            .map_err(|_| SecretsError::Crypto("local unwrap".into()))
    }
}

// ── Vault Transit–compatible HTTP ────────────────────────────────────

pub struct HttpTransitProvider {
    endpoint: String,
    token: String,
    key_id: String,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct TransitEncryptReq {
    plaintext: String, // base64
}

#[derive(Deserialize)]
struct TransitEncryptResp {
    data: TransitEncryptData,
}

#[derive(Deserialize)]
struct TransitEncryptData {
    ciphertext: String,
}

#[derive(Serialize)]
struct TransitDecryptReq {
    ciphertext: String,
}

#[derive(Deserialize)]
struct TransitDecryptResp {
    data: TransitDecryptData,
}

#[derive(Deserialize)]
struct TransitDecryptData {
    plaintext: String, // base64
}

#[async_trait]
impl KeyProvider for HttpTransitProvider {
    fn provider_name(&self) -> &str {
        "http"
    }
    fn key_id(&self) -> &str {
        &self.key_id
    }

    async fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>> {
        let url = format!(
            "{}/v1/transit/encrypt/{}",
            self.endpoint,
            urlencoding_lite(&self.key_id)
        );
        let body = TransitEncryptReq {
            plaintext: base64_encode(dek),
        };
        let mut req = self.client.post(&url).json(&body);
        if !self.token.is_empty() {
            req = req.header("X-Vault-Token", &self.token);
        }
        let res = req
            .send()
            .await
            .map_err(|e| SecretsError::Crypto(format!("vault encrypt: {e}")))?;
        if !res.status().is_success() {
            let t = res.text().await.unwrap_or_default();
            return Err(SecretsError::Crypto(format!("vault encrypt failed: {t}")));
        }
        let parsed: TransitEncryptResp = res
            .json()
            .await
            .map_err(|e| SecretsError::Crypto(format!("vault encrypt json: {e}")))?;
        Ok(parsed.data.ciphertext.into_bytes())
    }

    async fn unwrap_dek(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let ct = String::from_utf8_lossy(ciphertext).into_owned();
        let url = format!(
            "{}/v1/transit/decrypt/{}",
            self.endpoint,
            urlencoding_lite(&self.key_id)
        );
        let body = TransitDecryptReq { ciphertext: ct };
        let mut req = self.client.post(&url).json(&body);
        if !self.token.is_empty() {
            req = req.header("X-Vault-Token", &self.token);
        }
        let res = req
            .send()
            .await
            .map_err(|e| SecretsError::Crypto(format!("vault decrypt: {e}")))?;
        if !res.status().is_success() {
            let t = res.text().await.unwrap_or_default();
            return Err(SecretsError::Crypto(format!("vault decrypt failed: {t}")));
        }
        let parsed: TransitDecryptResp = res
            .json()
            .await
            .map_err(|e| SecretsError::Crypto(format!("vault decrypt json: {e}")))?;
        base64_decode(&parsed.data.plaintext)
    }
}

fn urlencoding_lite(s: &str) -> String {
    s.replace('/', "%2F")
}

fn base64_encode(data: &[u8]) -> String {
    use std::io::Write;
    let mut buf = Vec::new();
    {
        let mut enc = base64_writer(&mut buf);
        enc.write_all(data).ok();
        enc.finish().ok();
    }
    String::from_utf8(buf).unwrap_or_default()
}

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    base64_simd_decode(s).map_err(|e| SecretsError::Crypto(format!("b64: {e}")))
}

// Minimal base64 without extra crate dependency pressure — use engine from std via manual
fn base64_writer(buf: &mut Vec<u8>) -> Base64Writer<'_> {
    Base64Writer { buf, rem: [0; 3], rem_len: 0 }
}

struct Base64Writer<'a> {
    buf: &'a mut Vec<u8>,
    rem: [u8; 3],
    rem_len: usize,
}

impl<'a> std::io::Write for Base64Writer<'a> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0;
        while i < data.len() {
            self.rem[self.rem_len] = data[i];
            self.rem_len += 1;
            i += 1;
            if self.rem_len == 3 {
                let n = ((self.rem[0] as u32) << 16) | ((self.rem[1] as u32) << 8) | self.rem[2] as u32;
                self.buf.push(T[((n >> 18) & 63) as usize]);
                self.buf.push(T[((n >> 12) & 63) as usize]);
                self.buf.push(T[((n >> 6) & 63) as usize]);
                self.buf.push(T[(n & 63) as usize]);
                self.rem_len = 0;
            }
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> Base64Writer<'a> {
    fn finish(mut self) -> std::io::Result<()> {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        if self.rem_len == 1 {
            let n = (self.rem[0] as u32) << 16;
            self.buf.push(T[((n >> 18) & 63) as usize]);
            self.buf.push(T[((n >> 12) & 63) as usize]);
            self.buf.push(b'=');
            self.buf.push(b'=');
        } else if self.rem_len == 2 {
            let n = ((self.rem[0] as u32) << 16) | ((self.rem[1] as u32) << 8);
            self.buf.push(T[((n >> 18) & 63) as usize]);
            self.buf.push(T[((n >> 12) & 63) as usize]);
            self.buf.push(T[((n >> 6) & 63) as usize]);
            self.buf.push(b'=');
        }
        Ok(())
    }
}

fn base64_simd_decode(s: &str) -> std::result::Result<Vec<u8>, String> {
    fn val(c: u8) -> std::result::Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("bad b64 byte {c}")),
        }
    }
    let s = s.trim().as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = [0u8; 4];
    let mut n = 0;
    for &c in s {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        buf[n] = val(c)?;
        n += 1;
        if n == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
            n = 0;
        }
    }
    if n == 3 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
        out.push((buf[1] << 4) | (buf[2] >> 2));
    } else if n == 2 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
    }
    Ok(out)
}

/// Encode provider metadata + ciphertext into storage blob.
pub fn pack_wrapped(provider: &str, key_id: &str, ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + provider.len() + key_id.len() + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.push(1); // version
    let p = provider.as_bytes();
    out.push(p.len() as u8);
    out.extend_from_slice(p);
    let k = key_id.as_bytes();
    out.extend_from_slice(&(k.len() as u16).to_be_bytes());
    out.extend_from_slice(k);
    out.extend_from_slice(ciphertext);
    out
}

/// Decode storage blob → (provider, key_id, ciphertext). Legacy blobs without magic are treated as local raw.
pub fn unpack_wrapped(blob: &[u8]) -> Result<(String, String, Vec<u8>)> {
    if blob.len() >= 4 && &blob[..4] == MAGIC {
        if blob.len() < 8 {
            return Err(SecretsError::Crypto("wrapped blob short".into()));
        }
        let _version = blob[4];
        let plen = blob[5] as usize;
        let mut i = 6;
        if i + plen + 2 > blob.len() {
            return Err(SecretsError::Crypto("wrapped blob truncated".into()));
        }
        let provider = String::from_utf8_lossy(&blob[i..i + plen]).into_owned();
        i += plen;
        let klen = u16::from_be_bytes([blob[i], blob[i + 1]]) as usize;
        i += 2;
        if i + klen > blob.len() {
            return Err(SecretsError::Crypto("wrapped blob key truncated".into()));
        }
        let key_id = String::from_utf8_lossy(&blob[i..i + klen]).into_owned();
        i += klen;
        Ok((provider, key_id, blob[i..].to_vec()))
    } else {
        // Legacy: pure local AES wrap
        Ok(("local".into(), "legacy".into(), blob.to_vec()))
    }
}

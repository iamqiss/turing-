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
use tracing::{info, warn};

use crate::error::{Result, SecretsError};

const MAGIC: &[u8; 4] = b"TTK1";

#[async_trait]
pub trait KeyProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn key_id(&self) -> &str;
    async fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>>;
    async fn unwrap_dek(&self, ciphertext: &[u8]) -> Result<Vec<u8>>;

    /// Unwrap a DEK that was wrapped under `key_id` (the id stored beside it).
    ///
    /// The default only serves this provider's own key. Providers whose backend
    /// holds several keys (tt-kms) override it so old secrets stay readable
    /// after the active key changes.
    async fn unwrap_dek_for(&self, key_id: &str, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if key_id == self.key_id() {
            self.unwrap_dek(ciphertext).await
        } else {
            Err(SecretsError::Crypto(format!(
                "{} provider holds key {:?}, not {key_id:?}",
                self.provider_name(),
                self.key_id()
            )))
        }
    }
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
    /// Allow the `local` provider to run with the publicly known dev KEK when
    /// `local_key_hex` is empty. Lab only; default false.
    pub allow_insecure_dev_key: bool,
    /// Old local KEKs that existing secrets may still be sealed under, as
    /// `(key_id, 64-hex)`. Parse `TT_SECRETS_RETIRED_KEYS` with
    /// [`parse_retired_local_keys`]. Works with any active provider, so it is
    /// also the way to keep reading `local` secrets after moving to `tt`.
    pub retired_local_keys: Vec<(String, String)>,
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
            allow_insecure_dev_key: false,
            retired_local_keys: Vec::new(),
        }
    }
}

/// Active provider (used for every new seal) plus retired providers (read-only).
pub fn build_providers(
    cfg: &KmsConfig,
) -> Result<(
    std::sync::Arc<dyn KeyProvider>,
    Vec<std::sync::Arc<dyn KeyProvider>>,
)> {
    let active = build_provider(cfg)?;
    let mut retired: Vec<std::sync::Arc<dyn KeyProvider>> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (id, hex_key) in &cfg.retired_local_keys {
        validate_key_id(id)?;
        if hex_key.trim().is_empty() {
            return Err(SecretsError::Crypto(format!(
                "retired key {id:?} has an empty value"
            )));
        }
        if !seen.insert(id.clone()) {
            return Err(SecretsError::Crypto(format!("retired key id {id:?} listed twice")));
        }
        if active.provider_name() == "local" && active.key_id() == id {
            return Err(SecretsError::Crypto(format!(
                "retired key id {id:?} is also the active local key id"
            )));
        }
        let key = parse_local_key(hex_key, false)?;
        retired.push(std::sync::Arc::new(LocalKeyProvider {
            key,
            key_id: id.clone(),
        }));
    }
    if !retired.is_empty() {
        info!(count = retired.len(), "KMS: retired local keys loaded (read-only)");
    }
    Ok((active, retired))
}

fn validate_key_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(SecretsError::Crypto(format!(
            "invalid key id {id:?} (use 1-64 chars of A-Z a-z 0-9 - _ .)"
        )));
    }
    Ok(())
}

/// Parse `TT_SECRETS_RETIRED_KEYS`: comma-separated `key_id:64-hex` pairs.
/// Whitespace and empty entries are ignored. Values are validated when the
/// providers are built.
pub fn parse_retired_local_keys(s: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (id, key) = part.split_once(':').ok_or_else(|| {
            SecretsError::Crypto(
                "TT_SECRETS_RETIRED_KEYS entries must look like key_id:64hex".into(),
            )
        })?;
        let id = id.trim();
        validate_key_id(id)?;
        out.push((id.to_string(), key.trim().to_string()));
    }
    Ok(out)
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
            let key = parse_local_key(&cfg.local_key_hex, cfg.allow_insecure_dev_key)?;
            if cfg.local_key_hex.trim().is_empty() {
                warn!(
                    key_id = %cfg.key_id,
                    "KMS provider: local with the PUBLICLY KNOWN dev key (TT_ALLOW_INSECURE_DEV_KEY=1) — \
                     secrets are readable by anyone with the database; lab only"
                );
            } else {
                info!(key_id = %cfg.key_id, "KMS provider: local (lab only — use tt-kms on metal)");
            }
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
fn parse_local_key(hex_or_empty: &str, allow_insecure_dev_key: bool) -> Result<[u8; 32]> {
    let s = hex_or_empty.trim();
    if s.is_empty() {
        if !allow_insecure_dev_key {
            return Err(SecretsError::Crypto(
                "local KMS needs TT_SECRETS_KEY (64 hex chars; `openssl rand -hex 32`). \
                 For a throwaway lab only, set TT_ALLOW_INSECURE_DEV_KEY=1"
                    .into(),
            ));
        }
        // Publicly known key: lab only, and only on explicit opt-in.
        let mut out = [0u8; 32];
        out[..16].copy_from_slice(b"tt-dev-only-kek!");
        out[16..].copy_from_slice(b"-secrets-key!!!!"); // 16
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

/// String that is HMAC-signed for tt-kms. Binds method, path, body and moment:
/// `tt-kms-v2\n{ts}\n{nonce}\n{METHOD}\n{path}\n{hex(sha256(body))}`
///
/// Must stay identical to `signing_input` in `crates/tt-kms/src/api.rs`; both
/// crates assert the same golden vector.
fn signing_input(ts: &str, nonce: &str, method: &str, path: &str, body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "tt-kms-v2\n{ts}\n{nonce}\n{method}\n{path}\n{}",
        hex::encode(Sha256::digest(body))
    )
}

fn hmac_hex(secret: &str, input: &str) -> Result<String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .map_err(|e| SecretsError::Crypto(e.to_string()))?;
    mac.update(input.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

struct KmsAuth {
    ts: String,
    nonce: String,
    sig: String,
}

fn sign_request(secret: &str, method: &str, path: &str, body: &[u8]) -> Result<KmsAuth> {
    let ts = chrono::Utc::now().timestamp().to_string();
    let mut raw = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut raw);
    let nonce = hex::encode(raw);
    let sig = hmac_hex(secret, &signing_input(&ts, &nonce, method, path, body))?;
    Ok(KmsAuth { ts, nonce, sig })
}

impl TtKmsProvider {
    /// Unwrap under an explicit key id (tt-kms may hold several keys).
    async fn unwrap_with_key(&self, key_id: &str, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let body = TtUnwrapReq {
            key_id: Some(key_id.to_string()),
            ciphertext_b64: b64_enc(ciphertext),
        };
        let res = self.post_signed("/v1/unwrap", &body, "unwrap").await?;
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

    /// POST a JSON body, signed when a shared secret is configured.
    /// The signed bytes are exactly the bytes sent.
    async fn post_signed<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
        what: &str,
    ) -> Result<reqwest::Response> {
        let bytes = serde_json::to_vec(body)
            .map_err(|e| SecretsError::Crypto(format!("tt-kms {what} encode: {e}")))?;
        let url = format!("{}{}", self.endpoint, path);
        // sign the path the server will actually see (endpoint may carry a prefix)
        let signed_path = reqwest::Url::parse(&url)
            .map_err(|e| SecretsError::Crypto(format!("tt-kms endpoint: {e}")))?
            .path()
            .to_string();
        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json");
        if !self.auth_secret.is_empty() {
            let a = sign_request(&self.auth_secret, "POST", &signed_path, &bytes)?;
            req = req
                .header("x-tt-kms-timestamp", a.ts)
                .header("x-tt-kms-nonce", a.nonce)
                .header("x-tt-kms-signature", a.sig);
        }
        req.body(bytes)
            .send()
            .await
            .map_err(|e| SecretsError::Crypto(format!("tt-kms {what} unreachable: {e}")))
    }
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
        let body = TtWrapReq {
            key_id: Some(self.key_id.clone()),
            plaintext_b64: b64_enc(dek),
        };
        let res = self.post_signed("/v1/wrap", &body, "wrap").await?;
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
        self.unwrap_with_key(&self.key_id, ciphertext).await
    }

    async fn unwrap_dek_for(&self, key_id: &str, ciphertext: &[u8]) -> Result<Vec<u8>> {
        self.unwrap_with_key(key_id, ciphertext).await
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_refuses_to_start_without_a_key() {
        let cfg = KmsConfig {
            provider: "local".into(),
            ..KmsConfig::default()
        };
        let err = build_provider(&cfg).err().expect("must fail").to_string();
        assert!(err.contains("TT_SECRETS_KEY"), "{err}");
        // blank/whitespace is the same as unset
        let cfg = KmsConfig {
            provider: "local".into(),
            local_key_hex: "   ".into(),
            ..KmsConfig::default()
        };
        assert!(build_provider(&cfg).is_err());
        // empty provider string means "local" too
        let cfg = KmsConfig {
            provider: String::new(),
            ..KmsConfig::default()
        };
        assert!(build_provider(&cfg).is_err());
    }

    #[test]
    fn dev_key_requires_explicit_opt_in_and_is_the_documented_value() {
        assert!(parse_local_key("", false).is_err());
        let k = parse_local_key("", true).unwrap();
        assert_eq!(&k[..16], b"tt-dev-only-kek!");
        let cfg = KmsConfig {
            provider: "local".into(),
            allow_insecure_dev_key: true,
            ..KmsConfig::default()
        };
        assert!(build_provider(&cfg).is_ok());
    }

    #[test]
    fn retired_keys_env_format_is_parsed_and_validated() {
        let hex1 = "11".repeat(32);
        let hex2 = "22".repeat(32);
        let v = parse_retired_local_keys(&format!(" k1:{hex1} , k-2.b:{hex2},, ")).unwrap();
        assert_eq!(v, vec![("k1".into(), hex1), ("k-2.b".into(), hex2)]);
        assert!(parse_retired_local_keys("").unwrap().is_empty());
        assert!(parse_retired_local_keys("no-colon").is_err());
        assert!(parse_retired_local_keys(":abcd").is_err());
        assert!(parse_retired_local_keys("bad id:abcd").is_err());
        assert!(parse_retired_local_keys(&format!("{}:00", "a".repeat(65))).is_err());
    }

    fn local_cfg(active_id: &str) -> KmsConfig {
        KmsConfig {
            provider: "local".into(),
            local_key_hex: "aa".repeat(32),
            key_id: active_id.into(),
            ..KmsConfig::default()
        }
    }

    #[test]
    fn build_providers_loads_retired_keys_read_only() {
        let mut cfg = local_cfg("k2");
        cfg.retired_local_keys = vec![("k1".into(), "11".repeat(32))];
        let (active, retired) = build_providers(&cfg).unwrap();
        assert_eq!((active.provider_name(), active.key_id()), ("local", "k2"));
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].key_id(), "k1");
    }

    #[test]
    fn build_providers_rejects_bad_retired_entries() {
        let good = "11".repeat(32);
        for entries in [
            vec![("k1".to_string(), "".to_string())],
            vec![("k1".to_string(), "zz".repeat(32))],
            vec![("k1".to_string(), "11".to_string())],
            vec![("bad id".to_string(), good.clone())],
            vec![("k1".to_string(), good.clone()), ("k1".to_string(), good.clone())],
            vec![("k2".to_string(), good.clone())], // collides with active id
        ] {
            let mut cfg = local_cfg("k2");
            cfg.retired_local_keys = entries.clone();
            assert!(build_providers(&cfg).is_err(), "{entries:?}");
        }
    }

    #[test]
    fn retired_local_keys_work_when_migrating_to_tt() {
        let cfg = KmsConfig {
            provider: "tt".into(),
            retired_local_keys: vec![("local".into(), "11".repeat(32))],
            ..KmsConfig::default()
        };
        let (active, retired) = build_providers(&cfg).unwrap();
        assert_eq!(active.provider_name(), "tt");
        assert_eq!(retired[0].provider_name(), "local");
    }

    #[tokio::test]
    async fn default_unwrap_dek_for_only_serves_its_own_key() {
        let p = LocalKeyProvider {
            key: [4u8; 32],
            key_id: "mine".into(),
        };
        let wrapped = p.wrap_dek(&[1u8; 32]).await.unwrap();
        assert_eq!(p.unwrap_dek_for("mine", &wrapped).await.unwrap(), vec![1u8; 32]);
        let err = p.unwrap_dek_for("other", &wrapped).await.unwrap_err().to_string();
        assert!(err.contains("mine") && err.contains("other"), "{err}");
    }

    #[test]
    fn explicit_local_key_is_validated() {
        let ok = "11".repeat(32);
        assert_eq!(parse_local_key(&ok, false).unwrap(), [0x11u8; 32]);
        assert_eq!(parse_local_key(&format!("  {ok}\n"), false).unwrap(), [0x11u8; 32]);
        assert!(parse_local_key("11", false).is_err());
        assert!(parse_local_key(&"11".repeat(33), false).is_err());
        assert!(parse_local_key(&"zz".repeat(32), true).is_err());
    }

    #[test]
    fn kms_signature_golden_vector_matches_tt_kms() {
        // Keep identical to the vector in crates/tt-kms/src/api.rs.
        let secret = "unit-test-secret-unit-test-secret-32";
        let body = br#"{"key_id":"k1","ciphertext_b64":"AAAA"}"#;
        let input = signing_input("1761800000", "0123456789abcdef", "POST", "/v1/unwrap", body);
        assert_eq!(
            hmac_hex(secret, &input).unwrap(),
            "2c37d2438995e008f01b46c58f225849d8611314a20b0a9d388defbf235a7e60"
        );
    }

    #[test]
    fn sign_request_uses_fresh_nonce_and_binds_body() {
        let a = sign_request("s".repeat(32).as_str(), "POST", "/v1/wrap", b"{}").unwrap();
        let b = sign_request("s".repeat(32).as_str(), "POST", "/v1/wrap", b"{}").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_eq!(a.nonce.len(), 32);
        assert!(a.nonce.bytes().all(|c| c.is_ascii_alphanumeric()));
        let c = sign_request("s".repeat(32).as_str(), "POST", "/v1/wrap", b"{ }").unwrap();
        assert_ne!(a.sig, c.sig);
    }

    #[tokio::test]
    async fn local_wrap_unwrap_roundtrip() {
        let mut key = [0u8; 32];
        key[0] = 7;
        let p = LocalKeyProvider {
            key,
            key_id: "test".into(),
        };
        let dek = [9u8; 32];
        let wrapped = p.wrap_dek(&dek).await.unwrap();
        let open = p.unwrap_dek(&wrapped).await.unwrap();
        assert_eq!(open, dek);
    }

    #[test]
    fn pack_unpack_metadata() {
        let blob = pack_wrapped("tt", "scarlet-v1", b"ciphertext");
        let (prov, kid, ct) = unpack_wrapped(&blob).unwrap();
        assert_eq!(prov, "tt");
        assert_eq!(kid, "scarlet-v1");
        assert_eq!(ct, b"ciphertext");
    }
}

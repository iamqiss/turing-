//! Internal KMS HTTP API — bind to private network only on TT metal.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::KmsEngine;
use crate::error::KmsError;

type HmacSha256 = Hmac<Sha256>;

/// Max accepted clock difference between control plane and KMS.
pub const MAX_SKEW_SECS: i64 = 60;
/// wrap/unwrap bodies are tiny (a 32-byte DEK, base64'd).
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Upper bound on remembered nonces (memory cap).
const MAX_NONCES: usize = 100_000;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<KmsEngine>,
    /// Shared secret between control plane and tt-kms (HMAC).
    /// Empty = unauthenticated lab mode (the binary refuses this unless
    /// `TT_KMS_INSECURE_LAB=1`).
    pub auth_secret: String,
    /// Recently seen request nonces -> unix time when they may be forgotten.
    nonces: Arc<Mutex<HashMap<String, i64>>>,
}

impl AppState {
    pub fn new(engine: Arc<KmsEngine>, auth_secret: String) -> Self {
        Self {
            engine,
            auth_secret,
            nonces: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub fn router(state: AppState) -> Router {
    // Only key operations require a signature; /v1/health stays open for probes.
    let protected = Router::new()
        .route("/v1/wrap", post(wrap))
        .route("/v1/unwrap", post(unwrap))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_signature,
        ));
    Router::new()
        .route("/v1/health", get(health))
        .merge(protected)
        .with_state(state)
}

/// String that is HMAC-signed by the caller. It binds the request to one
/// method, path, body and moment:
///
/// `tt-kms-v2\n{ts}\n{nonce}\n{METHOD}\n{path}\n{hex(sha256(body))}`
///
/// The same function (with the same golden test vector) lives in
/// `tt-secrets/src/kms.rs`. Change both together.
pub fn signing_input(ts: &str, nonce: &str, method: &str, path: &str, body: &[u8]) -> String {
    format!(
        "tt-kms-v2\n{ts}\n{nonce}\n{method}\n{path}\n{}",
        hex::encode(Sha256::digest(body))
    )
}

/// Verify the signature headers. `now` is unix seconds (injected for tests).
fn verify_request(
    state: &AppState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
    now: i64,
) -> Result<(), StatusCode> {
    if state.auth_secret.is_empty() {
        return Ok(()); // lab mode
    }
    let header = |name: &str| -> Result<&str, StatusCode> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)
    };
    let sig_hex = header("x-tt-kms-signature")?;
    let ts_str = header("x-tt-kms-timestamp")?;
    let nonce = header("x-tt-kms-nonce")?;

    if nonce.len() < 16 || nonce.len() > 64 || !nonce.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let ts: i64 = ts_str.parse().map_err(|_| StatusCode::UNAUTHORIZED)?;
    if (now - ts).abs() > MAX_SKEW_SECS {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let sig = hex::decode(sig_hex).map_err(|_| StatusCode::UNAUTHORIZED)?;

    let mut mac = <HmacSha256 as Mac>::new_from_slice(state.auth_secret.as_bytes())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    mac.update(signing_input(ts_str, nonce, method, path, body).as_bytes());
    // constant-time comparison
    mac.verify_slice(&sig).map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Replay protection. Only authenticated nonces are stored, so an
    // unauthenticated caller cannot fill the cache.
    let mut seen = state
        .nonces
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    seen.retain(|_, forget_at| *forget_at > now);
    if seen.contains_key(nonce) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if seen.len() >= MAX_NONCES {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    // keep it for as long as the timestamp could still be accepted
    seen.insert(nonce.to_string(), ts + MAX_SKEW_SECS + 1);
    Ok(())
}

async fn require_signature(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.auth_secret.is_empty() {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    let bytes = match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    if let Err(code) = verify_request(
        &state,
        parts.method.as_str(),
        parts.uri.path(),
        &parts.headers,
        &bytes,
        chrono::Utc::now().timestamp(),
    ) {
        return code.into_response();
    }
    next.run(Request::from_parts(parts, Body::from(bytes))).await
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let locked = state.engine.is_locked().await;
    Json(serde_json::json!({
        "service": "tt-kms",
        "locked": locked,
        "default_key_id": state.engine.default_key_id().await,
    }))
}

#[derive(Debug, Deserialize)]
struct WrapBody {
    key_id: Option<String>,
    /// base64 DEK (32 bytes)
    plaintext_b64: String,
}

#[derive(Debug, Serialize)]
struct WrapResp {
    ciphertext_b64: String,
    key_id: String,
}

#[derive(Debug, Deserialize)]
struct UnwrapBody {
    key_id: Option<String>,
    ciphertext_b64: String,
}

#[derive(Debug, Serialize)]
struct UnwrapResp {
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

fn b64_dec(s: &str) -> Result<Vec<u8>, StatusCode> {
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
        buf[n] = val(c).ok_or(StatusCode::BAD_REQUEST)?;
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

fn map_err(e: KmsError) -> axum::response::Response {
    match e {
        KmsError::Locked => (StatusCode::SERVICE_UNAVAILABLE, "locked").into_response(),
        KmsError::UnknownKey(k) => (StatusCode::NOT_FOUND, k).into_response(),
        KmsError::Auth => StatusCode::UNAUTHORIZED.into_response(),
        KmsError::Invalid(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response(),
    }
}

async fn wrap(
    State(state): State<AppState>,
    Json(body): Json<WrapBody>,
) -> impl IntoResponse {
    let dek = match b64_dec(&body.plaintext_b64) {
        Ok(d) => d,
        Err(s) => return s.into_response(),
    };
    match state.engine.wrap(body.key_id.as_deref(), &dek).await {
        Ok(ct) => {
            let kid = body
                .key_id
                .clone()
                .or(state.engine.default_key_id().await)
                .unwrap_or_default();
            Json(WrapResp {
                ciphertext_b64: b64_enc(&ct),
                key_id: kid,
            })
            .into_response()
        }
        Err(e) => map_err(e),
    }
}

async fn unwrap(
    State(state): State<AppState>,
    Json(body): Json<UnwrapBody>,
) -> impl IntoResponse {
    let ct = match b64_dec(&body.ciphertext_b64) {
        Ok(d) => d,
        Err(s) => return s.into_response(),
    };
    match state.engine.unwrap(body.key_id.as_deref(), &ct).await {
        Ok(pt) => Json(UnwrapResp {
            plaintext_b64: b64_enc(&pt),
        })
        .into_response(),
        Err(e) => map_err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const SECRET: &str = "unit-test-secret-unit-test-secret-32";
    const NONCE: &str = "0123456789abcdef";

    fn sign(secret: &str, ts: i64, nonce: &str, method: &str, path: &str, body: &[u8]) -> String {
        let mut m = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
        m.update(signing_input(&ts.to_string(), nonce, method, path, body).as_bytes());
        hex::encode(m.finalize().into_bytes())
    }

    fn headers(ts: i64, nonce: &str, sig: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-tt-kms-timestamp", HeaderValue::from_str(&ts.to_string()).unwrap());
        h.insert("x-tt-kms-nonce", HeaderValue::from_str(nonce).unwrap());
        h.insert("x-tt-kms-signature", HeaderValue::from_str(sig).unwrap());
        h
    }

    fn state() -> AppState {
        AppState::new(KmsEngine::new_locked(), SECRET.into())
    }

    const NOW: i64 = 1_761_800_000;
    const BODY: &[u8] = br#"{"key_id":"k1","ciphertext_b64":"AAAA"}"#;

    #[test]
    fn golden_vector_matches_client_in_tt_secrets() {
        // Keep identical to the vector in crates/tt-secrets/src/kms.rs.
        assert_eq!(
            sign(SECRET, NOW, NONCE, "POST", "/v1/unwrap", BODY),
            "2c37d2438995e008f01b46c58f225849d8611314a20b0a9d388defbf235a7e60"
        );
    }

    #[test]
    fn accepts_a_valid_signature() {
        let sig = sign(SECRET, NOW, NONCE, "POST", "/v1/unwrap", BODY);
        verify_request(&state(), "POST", "/v1/unwrap", &headers(NOW, NONCE, &sig), BODY, NOW)
            .unwrap();
    }

    #[test]
    fn rejects_replayed_nonce() {
        let st = state();
        let sig = sign(SECRET, NOW, NONCE, "POST", "/v1/unwrap", BODY);
        let h = headers(NOW, NONCE, &sig);
        verify_request(&st, "POST", "/v1/unwrap", &h, BODY, NOW).unwrap();
        let r = verify_request(&st, "POST", "/v1/unwrap", &h, BODY, NOW + 1);
        assert_eq!(r, Err(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn nonce_is_forgotten_after_the_skew_window() {
        let st = state();
        let sig = sign(SECRET, NOW, NONCE, "POST", "/v1/unwrap", BODY);
        let h = headers(NOW, NONCE, &sig);
        verify_request(&st, "POST", "/v1/unwrap", &h, BODY, NOW).unwrap();
        // timestamp is now too old to be accepted anyway
        let later = NOW + MAX_SKEW_SECS + 5;
        assert_eq!(
            verify_request(&st, "POST", "/v1/unwrap", &h, BODY, later),
            Err(StatusCode::UNAUTHORIZED)
        );
        // and the cache entry was purged by the next authenticated request
        let n2 = "fedcba9876543210";
        let sig2 = sign(SECRET, later, n2, "POST", "/v1/unwrap", BODY);
        verify_request(&st, "POST", "/v1/unwrap", &headers(later, n2, &sig2), BODY, later).unwrap();
        assert_eq!(st.nonces.lock().unwrap().len(), 1);
    }

    #[test]
    fn rejects_tampered_body_path_and_method() {
        let sig = sign(SECRET, NOW, NONCE, "POST", "/v1/unwrap", BODY);
        let h = headers(NOW, NONCE, &sig);
        let bad = br#"{"key_id":"k2","ciphertext_b64":"AAAA"}"#;
        assert!(verify_request(&state(), "POST", "/v1/unwrap", &h, bad, NOW).is_err());
        assert!(verify_request(&state(), "POST", "/v1/wrap", &h, BODY, NOW).is_err());
        assert!(verify_request(&state(), "PUT", "/v1/unwrap", &h, BODY, NOW).is_err());
    }

    #[test]
    fn rejects_wrong_secret() {
        let sig = sign("some-other-secret-some-other-secret", NOW, NONCE, "POST", "/v1/unwrap", BODY);
        assert!(
            verify_request(&state(), "POST", "/v1/unwrap", &headers(NOW, NONCE, &sig), BODY, NOW)
                .is_err()
        );
    }

    #[test]
    fn rejects_stale_and_future_timestamps() {
        for ts in [NOW - MAX_SKEW_SECS - 1, NOW + MAX_SKEW_SECS + 1] {
            let sig = sign(SECRET, ts, NONCE, "POST", "/v1/unwrap", BODY);
            let r = verify_request(&state(), "POST", "/v1/unwrap", &headers(ts, NONCE, &sig), BODY, NOW);
            assert_eq!(r, Err(StatusCode::UNAUTHORIZED), "ts={ts}");
        }
        // edge of the window is fine
        let ts = NOW - MAX_SKEW_SECS;
        let sig = sign(SECRET, ts, NONCE, "POST", "/v1/unwrap", BODY);
        verify_request(&state(), "POST", "/v1/unwrap", &headers(ts, NONCE, &sig), BODY, NOW).unwrap();
    }

    #[test]
    fn rejects_missing_or_malformed_headers() {
        let st = state();
        let empty = HeaderMap::new();
        assert!(verify_request(&st, "POST", "/v1/unwrap", &empty, BODY, NOW).is_err());
        let sig = sign(SECRET, NOW, NONCE, "POST", "/v1/unwrap", BODY);
        // short / non-alphanumeric nonce
        let too_long = "a".repeat(65);
        for n in ["short", "0123456789abcde!", too_long.as_str()] {
            let r = verify_request(&st, "POST", "/v1/unwrap", &headers(NOW, n, &sig), BODY, NOW);
            assert!(r.is_err(), "nonce {n}");
        }
        // non-hex signature, odd-length signature
        for bad in ["zz", "abc", ""] {
            let r = verify_request(&st, "POST", "/v1/unwrap", &headers(NOW, NONCE, bad), BODY, NOW);
            assert!(r.is_err(), "sig {bad:?}");
        }
    }

    #[test]
    fn unauthenticated_requests_cannot_fill_the_nonce_cache() {
        let st = state();
        for i in 0..50 {
            let n = format!("{i:016}");
            let _ = verify_request(&st, "POST", "/v1/unwrap", &headers(NOW, &n, "00"), BODY, NOW);
        }
        assert_eq!(st.nonces.lock().unwrap().len(), 0);
    }

    #[test]
    fn empty_secret_is_open_lab_mode() {
        let st = AppState::new(KmsEngine::new_locked(), String::new());
        verify_request(&st, "POST", "/v1/unwrap", &HeaderMap::new(), BODY, NOW).unwrap();
    }

    // ---- end-to-end over a real socket: proves the middleware is actually mounted ----

    async fn spawn(secret: &str) -> std::net::SocketAddr {
        let engine = KmsEngine::new_locked();
        engine.unlock("k1", [7u8; 32]).await.unwrap();
        let app = router(AppState::new(engine, secret.into()));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(l, app).await.unwrap();
        });
        addr
    }

    async fn raw(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        extra: &[(&str, String)],
        body: &[u8],
    ) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (k, v) in extra {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        s.write_all(body).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf)
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }

    fn signed(path: &str, nonce: &str, body: &[u8]) -> Vec<(&'static str, String)> {
        let ts = chrono::Utc::now().timestamp();
        vec![
            ("x-tt-kms-timestamp", ts.to_string()),
            ("x-tt-kms-nonce", nonce.to_string()),
            ("x-tt-kms-signature", sign(SECRET, ts, nonce, "POST", path, body)),
        ]
    }

    #[tokio::test]
    async fn e2e_health_is_open_but_key_ops_require_a_signature() {
        let addr = spawn(SECRET).await;
        assert_eq!(raw(addr, "GET", "/v1/health", &[], b"").await, 200);

        let body = format!(r#"{{"key_id":"k1","plaintext_b64":"{}"}}"#, b64_enc(&[1u8; 32]));
        let body = body.as_bytes();
        assert_eq!(raw(addr, "POST", "/v1/wrap", &[], body).await, 401);
        assert_eq!(raw(addr, "POST", "/v1/unwrap", &[], body).await, 401);

        let h = signed("/v1/wrap", "aaaaaaaaaaaaaaaa", body);
        assert_eq!(raw(addr, "POST", "/v1/wrap", &h, body).await, 200);
        // exact replay
        assert_eq!(raw(addr, "POST", "/v1/wrap", &h, body).await, 401);
    }

    #[tokio::test]
    async fn e2e_signature_does_not_transfer_between_bodies_or_paths() {
        let addr = spawn(SECRET).await;
        let body_a = format!(r#"{{"key_id":"k1","plaintext_b64":"{}"}}"#, b64_enc(&[1u8; 32]));
        let body_b = format!(r#"{{"key_id":"k1","plaintext_b64":"{}"}}"#, b64_enc(&[2u8; 32]));
        let h = signed("/v1/wrap", "bbbbbbbbbbbbbbbb", body_a.as_bytes());
        assert_eq!(raw(addr, "POST", "/v1/wrap", &h, body_b.as_bytes()).await, 401);
        assert_eq!(raw(addr, "POST", "/v1/unwrap", &h, body_a.as_bytes()).await, 401);
    }

    #[tokio::test]
    async fn e2e_lab_mode_without_secret_is_open() {
        let addr = spawn("").await;
        let body = format!(r#"{{"key_id":"k1","plaintext_b64":"{}"}}"#, b64_enc(&[1u8; 32]));
        assert_eq!(raw(addr, "POST", "/v1/wrap", &[], body.as_bytes()).await, 200);
    }
}

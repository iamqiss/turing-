//! Internal KMS HTTP API — bind to private network only on TT metal.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::engine::KmsEngine;
use crate::error::KmsError;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<KmsEngine>,
    /// Shared secret between control plane and tt-kms (HMAC). Empty = open lab mode.
    pub auth_secret: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/wrap", post(wrap))
        .route("/v1/unwrap", post(unwrap))
        .with_state(state)
}

fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    if state.auth_secret.is_empty() {
        return Ok(()); // lab
    }
    let sig = headers
        .get("x-tt-kms-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let ts = headers
        .get("x-tt-kms-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(state.auth_secret.as_bytes())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    mac.update(ts.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());
    if expected != sig {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
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
    headers: HeaderMap,
    Json(body): Json<WrapBody>,
) -> impl IntoResponse {
    if let Err(s) = check_auth(&state, &headers) {
        return s.into_response();
    }
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
    headers: HeaderMap,
    Json(body): Json<UnwrapBody>,
) -> impl IntoResponse {
    if let Err(s) = check_auth(&state, &headers) {
        return s.into_response();
    }
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

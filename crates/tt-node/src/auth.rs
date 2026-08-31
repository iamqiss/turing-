//! Verify control-plane HMAC on commands.
//!
//! Env: TT_NODE_AGENT_TOKEN must match the token issued at registration.

use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn verify_headers(headers: &HeaderMap, body: &[u8]) -> Result<(), StatusCode> {
    let expected = match std::env::var("TT_NODE_AGENT_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            // Dev: no token configured — accept (lab only)
            tracing::warn!("TT_NODE_AGENT_TOKEN unset — accepting unsigned commands");
            return Ok(());
        }
    };
    let ts = headers
        .get("x-tt-timestamp")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let sig = headers
        .get("x-tt-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > 300 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut mac = HmacSha256::new_from_slice(expected.as_bytes()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected_sig = hex::encode(mac.finalize().into_bytes());
    if expected_sig != sig {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

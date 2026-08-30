//! Stripe webhook + billing helpers.
//!
//! Verifies `Stripe-Signature` when `stripe_webhook_secret` is set.
//! Updates org seat_limit from subscription item quantity.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tracing::{info, warn};

use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/billing/stripe/webhook", post(stripe_webhook))
}

async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let secret = state.config.stripe_webhook_secret.clone();
    if !secret.is_empty() {
        let sig = headers
            .get("Stripe-Signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !verify_stripe_signature(&secret, sig, &body) {
            warn!("stripe webhook signature mismatch");
            return StatusCode::BAD_REQUEST.into_response();
        }
    } else {
        warn!("stripe_webhook_secret empty – accepting webhook without verify (dev only)");
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let event_type = payload
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    info!(%event_type, "stripe webhook");

    match event_type {
        "customer.subscription.updated" | "customer.subscription.created" => {
            handle_subscription(&state, &payload).await;
        }
        "customer.subscription.deleted" => {
            if let Some(obj) = payload.pointer("/data/object") {
                let customer = obj.get("customer").and_then(|c| c.as_str()).unwrap_or("");
                let _ = state
                    .orgs
                    .set_billing_from_stripe(customer, None, 1, "canceled")
                    .await;
            }
        }
        _ => {}
    }

    Json(serde_json::json!({ "received": true })).into_response()
}

async fn handle_subscription(state: &AppState, payload: &serde_json::Value) {
    let obj = match payload.pointer("/data/object") {
        Some(o) => o,
        None => return,
    };
    let customer = obj.get("customer").and_then(|c| c.as_str()).unwrap_or("");
    let sub_id = obj.get("id").and_then(|c| c.as_str());
    let status = obj.get("status").and_then(|c| c.as_str()).unwrap_or("active");
    // quantity on first item
    let qty = obj
        .pointer("/items/data/0/quantity")
        .and_then(|q| q.as_u64())
        .unwrap_or(1) as u32;
    match state
        .orgs
        .set_billing_from_stripe(customer, sub_id, qty.max(1), status)
        .await
    {
        Ok(Some(org)) => info!(org = %org.slug, seats = qty, "org seats updated from stripe"),
        Ok(None) => warn!(%customer, "no org linked to stripe customer"),
        Err(e) => warn!(error = %e, "stripe seat update failed"),
    }
}

/// Stripe-Signature: t=timestamp,v1=hex
fn verify_stripe_signature(secret: &str, header: &str, body: &[u8]) -> bool {
    let mut timestamp = None;
    let mut v1 = None;
    for part in header.split(',') {
        let mut kv = part.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("t"), Some(t)) => timestamp = Some(t),
            (Some("v1"), Some(v)) => v1 = Some(v),
            _ => {}
        }
    }
    let (Some(ts), Some(sig_hex)) = (timestamp, v1) else {
        return false;
    };
    let signed = format!("{ts}.{}", String::from_utf8_lossy(body));
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(signed.as_bytes());
    let expected = mac.finalize().into_bytes();
    let Ok(provided) = hex::decode(sig_hex) else {
        return false;
    };
    // constant-time-ish compare
    expected.as_slice() == provided.as_slice()
}

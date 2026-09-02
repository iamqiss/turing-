//! Billing API — seats (Stripe) + usage meters + platform finance.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tracing::{info, warn};
use tt_billing::{Meter, PlanId};

use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/billing/stripe/webhook", post(stripe_webhook))
        .route("/api/billing/catalog", get(catalog))
        .route("/api/orgs/:slug/billing/account", get(org_billing))
        .route("/api/orgs/:slug/billing/invoice-preview", get(invoice_preview))
        .route("/api/orgs/:slug/billing/usage", post(record_usage))
        .route("/api/orgs/:slug/billing/invoices/issue", post(issue_invoice))
        .route("/api/orgs/:slug/billing/invoices/pay", post(mark_paid))
        .route("/api/platform/billing/finance", get(platform_finance))
}

async fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<tt_auth::AuthContext, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.trim()))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .principal_resolver
        .resolve_bearer(token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

async fn catalog(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.billing.catalog()).into_response()
}

async fn org_billing(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if state
        .orgs
        .require_member(org.id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let acct = match state.billing.ensure_account(org.id, PlanId::Team, org.seat_limit).await {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "billing account");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    Json(serde_json::json!({
        "org_id": org.id.to_string(),
        "plan": acct.plan_id.as_str(),
        "status": acct.status,
        "seat_quantity": acct.seat_quantity,
        "seat_limit": org.seat_limit,
        "currency": acct.currency,
        "stripe_customer_id": acct.stripe_customer_id,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct PeriodQuery {
    period: Option<String>,
}

async fn invoice_preview(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(q): Query<PeriodQuery>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if state
        .orgs
        .require_member(org.id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    match state.billing.invoice_preview(org.id, q.period.as_deref()).await {
        Ok(inv) => Json(inv).into_response(),
        Err(e) => {
            warn!(error = %e, "invoice preview");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct UsageBody {
    meter: String,
    quantity: f64,
    resource_id: Option<String>,
}

async fn record_usage(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UsageBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    // Only platform operators or internal services should post usage in production.
    // Lab: org admin can post for testing.
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if state
        .orgs
        .require_admin(org.id, ctx.principal_id())
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(meter) = Meter::parse(&body.meter) else {
        return (StatusCode::BAD_REQUEST, "unknown meter").into_response();
    };
    match state
        .billing
        .record_usage(org.id, meter, body.quantity, body.resource_id)
        .await
    {
        Ok(ev) => (StatusCode::CREATED, Json(ev)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn platform_finance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PeriodQuery>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let ops = state.config.platform_operator_id_list();
    let pid = ctx.principal_id().to_string();
    if !ops.iter().any(|id| id == &pid) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let period = q.period.unwrap_or_else(tt_billing::BillingService::period_ym_now);
    match state.billing.platform_finance(&period).await {
        Ok(s) => Json(s).into_response(),
        Err(e) => {
            warn!(error = %e, "platform finance");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
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
                let _ = state
                    .billing
                    .apply_stripe_seats(customer, None, 1, "canceled", PlanId::Free)
                    .await;
            }
        }
        "invoice.paid" | "invoice.payment_failed" | "charge.succeeded" | "charge.failed" => {
            handle_stripe_cash(&state, event_type, &payload).await;
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
    let qty = obj
        .pointer("/items/data/0/quantity")
        .and_then(|q| q.as_u64())
        .unwrap_or(1) as u32;
    let _ = state
        .orgs
        .set_billing_from_stripe(customer, sub_id, qty.max(1), status)
        .await;
    let _ = state
        .billing
        .apply_stripe_seats(customer, sub_id, qty.max(1), status, PlanId::Team)
        .await;
}


async fn handle_stripe_cash(state: &AppState, event_type: &str, payload: &serde_json::Value) {
    let event_id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let obj = payload.pointer("/data/object").cloned().unwrap_or(serde_json::json!({}));
    let customer = obj.get("customer").and_then(|c| c.as_str());
    // Stripe amounts are in cents → micros (1 cent = 10_000 micros)
    let cents = obj
        .get("amount_paid")
        .or_else(|| obj.get("amount"))
        .and_then(|a| a.as_i64())
        .unwrap_or(0);
    let amount_micros = cents.saturating_mul(10_000);
    let currency = obj
        .get("currency")
        .and_then(|c| c.as_str())
        .unwrap_or("usd");
    let status = obj.get("status").and_then(|s| s.as_str());
    match state
        .billing
        .record_stripe_event(
            event_id,
            event_type,
            customer,
            amount_micros,
            currency,
            status,
            payload.clone(),
        )
        .await
    {
        Ok(org) => info!(%event_type, ?org, amount_micros, "stripe cash reconciled"),
        Err(e) => warn!(error = %e, "stripe cash reconcile failed"),
    }
}

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
    let Ok(mut mac) = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(signed.as_bytes());
    let expected = mac.finalize().into_bytes();
    let Ok(provided) = hex::decode(sig_hex) else {
        return false;
    };
    expected.as_slice() == provided.as_slice()
}


#[derive(Deserialize)]
struct IssueBody {
    period: Option<String>,
}

async fn issue_invoice(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<IssueBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if state
        .orgs
        .require_permission(
            org.id,
            ctx.principal_id(),
            tt_core::domain::OrgPermission::ManageBilling,
        )
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    match state.billing.issue_invoice(org.id, body.period.as_deref()).await {
        Ok(inv) => (StatusCode::CREATED, Json(inv)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct PayBody {
    period: String,
    /// Optional manual cash (USD micros); defaults to invoice total
    cash_micros: Option<i64>,
}

async fn mark_paid(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PayBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if state
        .orgs
        .require_permission(
            org.id,
            ctx.principal_id(),
            tt_core::domain::OrgPermission::ManageBilling,
        )
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let cash = body.cash_micros.unwrap_or(0);
    match state
        .billing
        .mark_invoice_paid(org.id, &body.period, cash, None)
        .await
    {
        Ok(()) => Json(serde_json::json!({ "paid": true, "period": body.period })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

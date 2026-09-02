//! Platform superadmin + org cloud administration.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::{info, warn};
use tt_billing::PlanId;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/platform/admin/me", get(admin_me))
        .route("/api/platform/admin/orgs", get(list_orgs))
        .route("/api/platform/admin/orgs/:slug", get(get_org))
        .route("/api/platform/admin/orgs/:slug/suspend", post(suspend_org))
        .route("/api/platform/admin/orgs/:slug/unsuspend", post(unsuspend_org))
        .route("/api/platform/admin/orgs/:slug/plan", post(set_plan))
        .route("/api/platform/admin/finance", get(finance_dashboard))
        .route("/api/platform/admin/finance/ledger", get(finance_ledger))
        .route("/api/platform/admin/audit", get(list_audit))
        .route("/api/platform/admin/expenses", post(add_expense).get(list_expenses))
        .route("/api/platform/admin/jobs/git-storage-scan", post(git_storage_scan))
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

async fn write_audit(
    state: &AppState,
    actor: tt_core::ids::PrincipalId,
    action: &str,
    target_type: &str,
    target_id: Option<&str>,
    metadata: serde_json::Value,
) {
    let id = uuid::Uuid::now_v7();
    if let Err(e) = sqlx::query(
        r#"INSERT INTO admin_audit_log (id, actor_id, action, target_type, target_id, metadata, created_at)
           VALUES ($1,$2,$3,$4,$5,$6,NOW())"#,
    )
    .bind(id)
    .bind(actor.as_uuid())
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(metadata)
    .execute(&state.db)
    .await
    {
        warn!(error = %e, "admin audit write failed");
    }
}

fn require_superadmin(state: &AppState, ctx: &tt_auth::AuthContext) -> Result<(), StatusCode> {
    let pid = ctx.principal_id().to_string();
    if state.config.is_superadmin(&pid) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

async fn admin_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let pid = ctx.principal_id().to_string();
    let is_sa = state.config.is_superadmin(&pid);
    Json(serde_json::json!({
        "principal_id": pid,
        "superadmin": is_sa,
        "platform_operator": state.config.platform_operator_id_list().iter().any(|id| id == &pid),
    }))
    .into_response()
}

async fn list_orgs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Lightweight scan — full directory comes with admin store later
    match sqlx::query_as::<_, (uuid::Uuid, String, String, i32, Option<String>)>(
        r#"SELECT id, name, slug, seat_limit, billing_customer_id FROM orgs ORDER BY created_at DESC LIMIT 200"#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let list: Vec<_> = rows
                .into_iter()
                .map(|(id, name, slug, seats, cust)| {
                    serde_json::json!({
                        "id": id.to_string(),
                        "name": name,
                        "slug": slug,
                        "seat_limit": seats,
                        "billing_customer_id": cust,
                    })
                })
                .collect();
            Json(list).into_response()
        }
        Err(e) => {
            warn!(error = %e, "admin list orgs");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_org(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let snap = state.orgs.billing(org.id).await.ok();
    let acct = state.billing.get_account(org.id).await.ok();
    let inv = state.billing.invoice_preview(org.id, None).await.ok();
    Json(serde_json::json!({
        "org": {
            "id": org.id.to_string(),
            "name": org.name,
            "slug": org.slug,
            "seat_limit": org.seat_limit,
            "billing_customer_id": org.billing_customer_id,
        },
        "seats": snap,
        "billing_account": acct,
        "invoice_preview": inv,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SuspendBody {
    reason: Option<String>,
}

async fn suspend_org(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SuspendBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let seats = org.seat_limit.max(1);
    let plan = state
        .billing
        .get_account(org.id)
        .await
        .map(|a| a.plan_id)
        .unwrap_or(PlanId::Team);
    let _ = state
        .billing
        .set_plan(org.id, plan, seats, "suspended")
        .await;
    write_audit(
        &state,
        ctx.principal_id(),
        "org.suspend",
        "org",
        Some(&org.id.to_string()),
        serde_json::json!({ "slug": slug, "reason": body.reason }),
    )
    .await;
    info!(slug = %slug, by = %ctx.principal_id(), "superadmin suspended org");
    Json(serde_json::json!({ "slug": slug, "status": "suspended" })).into_response()
}

async fn unsuspend_org(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let seats = org.seat_limit.max(1);
    let plan = state
        .billing
        .get_account(org.id)
        .await
        .map(|a| a.plan_id)
        .unwrap_or(PlanId::Team);
    let _ = state.billing.set_plan(org.id, plan, seats, "active").await;
    write_audit(
        &state,
        ctx.principal_id(),
        "org.unsuspend",
        "org",
        Some(&org.id.to_string()),
        serde_json::json!({ "slug": slug }),
    )
    .await;
    info!(slug = %slug, by = %ctx.principal_id(), "superadmin unsuspended org");
    Json(serde_json::json!({ "slug": slug, "status": "active" })).into_response()
}

#[derive(Deserialize)]
struct PlanBody {
    plan: String,
    seat_quantity: Option<u32>,
}

async fn set_plan(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PlanBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(plan) = PlanId::parse(&body.plan) else {
        return (StatusCode::BAD_REQUEST, "unknown plan").into_response();
    };
    let org = match state.orgs.get_by_slug(&slug).await {
        Ok(o) => o,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let seats = body.seat_quantity.unwrap_or(org.seat_limit.max(1));
    let acct = match state.billing.set_plan(org.id, plan, seats, "active").await {
        Ok(a) => a,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let _ = state.orgs.set_seat_limit(org.id, ctx.principal_id(), seats).await;
    write_audit(
        &state,
        ctx.principal_id(),
        "org.set_plan",
        "org",
        Some(&org.id.to_string()),
        serde_json::json!({ "slug": slug, "plan": plan.as_str(), "seats": seats }),
    )
    .await;
    info!(slug = %slug, plan = %plan.as_str(), seats, "superadmin set plan");
    Json(serde_json::json!({
        "slug": slug,
        "plan": plan.as_str(),
        "seat_quantity": seats,
        "account_status": acct.status,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct PeriodQuery {
    period: Option<String>,
}

async fn finance_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PeriodQuery>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let period = q
        .period
        .unwrap_or_else(tt_billing::BillingService::period_ym_now);
    match state.billing.platform_finance(&period).await {
        Ok(snap) => Json(serde_json::json!({
            "period_ym": snap.period_ym,
            "revenue": {
                "seat_micros": snap.seat_revenue_micros,
                "usage_micros": snap.usage_revenue_micros,
                "total_micros": snap.total_revenue_micros,
            },
            "mrr_micros": snap.mrr_micros,
            "arr_micros": snap.arr_micros,
            "expenses": {
                "total_micros": snap.expenses_micros,
                "by_category": snap.expenses_by_category.iter().map(|(c,a)| {
                    serde_json::json!({"category": c, "amount_micros": a})
                }).collect::<Vec<_>>(),
            },
            "gross_margin_micros": snap.gross_margin_micros,
            "gross_margin_bps": snap.gross_margin_bps,
            "tax": {
                "name": snap.tax_name,
                "rate_bps": snap.tax_rate_bps,
                "estimated_on_revenue_micros": snap.estimated_tax_micros,
            },
            "invoices": {
                "count": snap.invoice_count,
                "paid_micros": snap.invoice_paid_micros,
            },
            "stripe_cash_micros": snap.stripe_cash_micros,
            "active_orgs": snap.active_orgs,
            "active_seats": snap.active_seats,
            "currency": snap.currency,
            "notes": [
                "MRR/ARR from current period run-rate (catalog seats + metered usage)",
                "Expenses are platform COGS/opex you book (metal, power, network, ...)",
                "Connect Stripe for cash-collected revenue reconciliation",
            ],
        }))
        .into_response(),
        Err(e) => {
            warn!(error = %e, "finance dashboard");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn finance_ledger(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PeriodQuery>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let period = q.period.unwrap_or_else(tt_billing::BillingService::period_ym_now);
    match sqlx::query_as::<_, (uuid::Uuid, Option<uuid::Uuid>, String, i64, String, Option<String>, Option<String>)>(
        r#"SELECT id, org_id, entry_type, amount_micros, currency, description, period_ym
           FROM billing_ledger_entries
           WHERE period_ym = $1 OR $1 IS NULL
           ORDER BY created_at DESC LIMIT 200"#,
    )
    .bind(&period)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let list: Vec<_> = rows
                .into_iter()
                .map(|(id, org, et, amt, cur, desc, py)| {
                    serde_json::json!({
                        "id": id.to_string(),
                        "org_id": org.map(|o| o.to_string()),
                        "entry_type": et,
                        "amount_micros": amt,
                        "currency": cur,
                        "description": desc,
                        "period_ym": py,
                    })
                })
                .collect();
            Json(list).into_response()
        }
        Err(e) => {
            // table may be empty
            warn!(error = %e, "ledger");
            Json(serde_json::json!([])).into_response()
        }
    }
}


async fn list_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    match sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, Option<String>, serde_json::Value, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT id, actor_id, action, target_type, target_id, metadata, created_at
           FROM admin_audit_log ORDER BY created_at DESC LIMIT 200"#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let list: Vec<_> = rows
                .into_iter()
                .map(|(id, actor, action, tt, tid, meta, at)| {
                    serde_json::json!({
                        "id": id.to_string(),
                        "actor_id": actor.to_string(),
                        "action": action,
                        "target_type": tt,
                        "target_id": tid,
                        "metadata": meta,
                        "created_at": at.to_rfc3339(),
                    })
                })
                .collect();
            Json(list).into_response()
        }
        Err(e) => {
            warn!(error = %e, "list audit");
            Json(serde_json::json!([])).into_response()
        }
    }
}


#[derive(Deserialize)]
struct ExpenseBody {
    period: Option<String>,
    category: String,
    /// USD micros (1_000_000 = $1)
    amount_micros: i64,
    vendor: Option<String>,
    description: Option<String>,
}

async fn add_expense(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ExpenseBody>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let period = body
        .period
        .unwrap_or_else(tt_billing::BillingService::period_ym_now);
    match state
        .billing
        .record_expense(
            &period,
            &body.category,
            body.amount_micros,
            body.vendor,
            body.description,
            Some(*ctx.principal_id().as_uuid()),
        )
        .await
    {
        Ok(id) => {
            write_audit(
                &state,
                ctx.principal_id(),
                "finance.expense",
                "platform",
                Some(&id.to_string()),
                serde_json::json!({ "period": period, "category": body.category, "amount_micros": body.amount_micros }),
            )
            .await;
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id.to_string(), "period_ym": period }))).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn list_expenses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PeriodQuery>,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let period = q.period.unwrap_or_else(tt_billing::BillingService::period_ym_now);
    match sqlx::query_as::<_, (uuid::Uuid, String, String, i64, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT id, period_ym, category, amount_micros, currency, vendor, description, created_at
           FROM billing_expenses WHERE period_ym = $1 ORDER BY created_at DESC LIMIT 200"#,
    )
    .bind(&period)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let list: Vec<_> = rows.into_iter().map(|(id, py, cat, amt, cur, ven, desc, at)| {
                serde_json::json!({
                    "id": id.to_string(),
                    "period_ym": py,
                    "category": cat,
                    "amount_micros": amt,
                    "currency": cur,
                    "vendor": ven,
                    "description": desc,
                    "created_at": at.to_rfc3339(),
                })
            }).collect();
            Json(list).into_response()
        }
        Err(e) => {
            warn!(error = %e, "list expenses");
            Json(serde_json::json!([])).into_response()
        }
    }
}


async fn git_storage_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_auth(&state, &headers).await {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if require_superadmin(&state, &ctx).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let rows: Vec<(uuid::Uuid, uuid::Uuid)> = match sqlx::query_as(
        "SELECT id, org_id FROM workspaces WHERE org_id IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "git scan workspaces");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let pairs: Vec<(tt_core::ids::WorkspaceId, tt_core::ids::OrgId)> = rows
        .into_iter()
        .map(|(ws, org)| {
            (
                tt_core::ids::WorkspaceId::from_uuid(ws),
                tt_core::ids::OrgId::from_uuid(org),
            )
        })
        .collect();
    let root = state.config.git_repos_root.clone();
    let usages = tt_git::storage_scan::scan_repos_root(std::path::Path::new(&root), &pairs);
    let period = tt_billing::BillingService::period_ym_now();
    let mut orgs = 0u32;
    let mut bytes_total = 0u64;
    for u in &usages {
        orgs += 1;
        bytes_total = bytes_total.saturating_add(u.bytes);
        let gb = tt_git::storage_scan::bytes_to_gb(u.bytes);
        let _ = state
            .billing
            .emit_git_storage_gb(u.org_id, gb, None)
            .await;
        let _ = sqlx::query(
            r#"INSERT INTO git_storage_snapshots (org_id, period_ym, bytes_total, repo_count, scanned_at)
               VALUES ($1,$2,$3,$4,NOW())
               ON CONFLICT (org_id, period_ym) DO UPDATE SET
                 bytes_total = EXCLUDED.bytes_total,
                 repo_count = EXCLUDED.repo_count,
                 scanned_at = NOW()"#,
        )
        .bind(u.org_id.as_uuid())
        .bind(&period)
        .bind(u.bytes as i64)
        .bind(u.repo_count as i32)
        .execute(&state.db)
        .await;
    }
    write_audit(
        &state,
        ctx.principal_id(),
        "jobs.git_storage_scan",
        "platform",
        None,
        serde_json::json!({ "orgs": orgs, "bytes_total": bytes_total, "period": period }),
    )
    .await;
    Json(serde_json::json!({
        "period_ym": period,
        "orgs_scanned": orgs,
        "bytes_total": bytes_total,
        "gb_total": tt_git::storage_scan::bytes_to_gb(bytes_total),
    }))
    .into_response()
}

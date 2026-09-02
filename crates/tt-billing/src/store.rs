use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use tt_core::ids::OrgId;
use uuid::Uuid;

use crate::catalog::{Meter, PlanId};
use crate::domain::{BillingAccount, ManagedKind, ManagedResource, UsageEvent, UsageMonthly};
use crate::error::{BillingError, Result};

#[async_trait]
pub trait BillingStore: Send + Sync {
    async fn upsert_account(&self, acct: &BillingAccount) -> Result<()>;
    async fn get_account(&self, org_id: OrgId) -> Result<Option<BillingAccount>>;
    async fn get_account_by_stripe_customer(&self, customer: &str) -> Result<Option<BillingAccount>>;
    async fn record_usage(&self, ev: &UsageEvent) -> Result<()>;
    async fn sum_usage(&self, org_id: OrgId, period_ym: &str, meter: Meter) -> Result<f64>;
    async fn upsert_monthly(&self, row: &UsageMonthly) -> Result<()>;
    async fn list_monthly(&self, org_id: OrgId, period_ym: &str) -> Result<Vec<UsageMonthly>>;
    async fn insert_managed(&self, r: &ManagedResource) -> Result<()>;
    async fn list_managed_active(&self, org_id: OrgId) -> Result<Vec<ManagedResource>>;
    async fn platform_sums(&self, period_ym: &str) -> Result<(i64, i64, u32, u32)>;
    async fn insert_expense(
        &self,
        id: Uuid,
        period_ym: &str,
        category: &str,
        amount_micros: i64,
        currency: &str,
        vendor: Option<&str>,
        description: Option<&str>,
        created_by: Option<Uuid>,
    ) -> Result<()>;
    async fn sum_expenses(&self, period_ym: &str) -> Result<(i64, Vec<(String, i64)>)>;
    async fn tax_settings(&self) -> Result<(String, i32)>;
    async fn invoice_stats(&self, period_ym: &str) -> Result<(u32, i64)>;
    async fn upsert_invoice(
        &self,
        id: Uuid,
        org_id: OrgId,
        period_ym: &str,
        status: &str,
        seat_amount_micros: i64,
        usage_amount_micros: i64,
        tax_amount_micros: i64,
        total_micros: i64,
        stripe_invoice_id: Option<&str>,
    ) -> Result<()>;
    async fn mark_invoice_paid(
        &self,
        org_id: OrgId,
        period_ym: &str,
        cash_micros: i64,
        stripe_invoice_id: Option<&str>,
    ) -> Result<()>;
    async fn insert_stripe_event(
        &self,
        id: Uuid,
        stripe_event_id: &str,
        event_type: &str,
        customer: Option<&str>,
        org_id: Option<OrgId>,
        amount_micros: i64,
        currency: &str,
        status: Option<&str>,
        period_ym: &str,
        raw: serde_json::Value,
    ) -> Result<()>;
    async fn sum_stripe_cash(&self, period_ym: &str) -> Result<i64>;

}


pub struct PostgresBillingStore {
    pool: PgPool,
}

impl PostgresBillingStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_plan(s: &str) -> PlanId {
    PlanId::parse(s).unwrap_or(PlanId::Team)
}

fn parse_meter(s: &str) -> Result<Meter> {
    Meter::parse(s).ok_or_else(|| BillingError::Invalid(format!("unknown meter {s}")))
}

#[async_trait]
impl BillingStore for PostgresBillingStore {
    async fn upsert_account(&self, acct: &BillingAccount) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO billing_accounts (
                org_id, plan_id, stripe_customer_id, stripe_subscription_id, currency, status,
                seat_quantity, current_period_start, current_period_end, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,NOW(),NOW())
            ON CONFLICT (org_id) DO UPDATE SET
                plan_id = EXCLUDED.plan_id,
                stripe_customer_id = EXCLUDED.stripe_customer_id,
                stripe_subscription_id = EXCLUDED.stripe_subscription_id,
                currency = EXCLUDED.currency,
                status = EXCLUDED.status,
                seat_quantity = EXCLUDED.seat_quantity,
                current_period_start = EXCLUDED.current_period_start,
                current_period_end = EXCLUDED.current_period_end,
                updated_at = NOW()
            "#,
        )
        .bind(acct.org_id.as_uuid())
        .bind(acct.plan_id.as_str())
        .bind(&acct.stripe_customer_id)
        .bind(&acct.stripe_subscription_id)
        .bind(&acct.currency)
        .bind(&acct.status)
        .bind(acct.seat_quantity as i32)
        .bind(acct.current_period_start)
        .bind(acct.current_period_end)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(())
    }

    async fn get_account(&self, org_id: OrgId) -> Result<Option<BillingAccount>> {
        let row: Option<(Uuid, String, Option<String>, Option<String>, String, String, i32, Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>)> =
            sqlx::query_as(
                r#"SELECT org_id, plan_id, stripe_customer_id, stripe_subscription_id, currency, status,
                          seat_quantity, current_period_start, current_period_end
                   FROM billing_accounts WHERE org_id = $1"#,
            )
            .bind(org_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| BillingError::Other(e.into()))?;
        Ok(row.map(|(org, plan, sc, ss, cur, st, seats, ps, pe)| BillingAccount {
            org_id: OrgId::from_uuid(org),
            plan_id: parse_plan(&plan),
            stripe_customer_id: sc,
            stripe_subscription_id: ss,
            currency: cur,
            status: st,
            seat_quantity: seats.max(0) as u32,
            current_period_start: ps,
            current_period_end: pe,
        }))
    }

    async fn get_account_by_stripe_customer(&self, customer: &str) -> Result<Option<BillingAccount>> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT org_id FROM billing_accounts WHERE stripe_customer_id = $1 LIMIT 1",
        )
        .bind(customer)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        match row {
            Some((id,)) => self.get_account(OrgId::from_uuid(id)).await,
            None => Ok(None),
        }
    }

    async fn record_usage(&self, ev: &UsageEvent) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO billing_usage_events
              (id, org_id, meter, quantity, unit, resource_id, period_ym, recorded_at, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'{}'::jsonb)
            "#,
        )
        .bind(ev.id)
        .bind(ev.org_id.as_uuid())
        .bind(ev.meter.as_str())
        .bind(ev.quantity)
        .bind(&ev.unit)
        .bind(&ev.resource_id)
        .bind(&ev.period_ym)
        .bind(ev.recorded_at)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(())
    }

    async fn sum_usage(&self, org_id: OrgId, period_ym: &str, meter: Meter) -> Result<f64> {
        let row: (Option<f64>,) = sqlx::query_as(
            r#"SELECT COALESCE(SUM(quantity), 0)::float8 FROM billing_usage_events
               WHERE org_id = $1 AND period_ym = $2 AND meter = $3"#,
        )
        .bind(org_id.as_uuid())
        .bind(period_ym)
        .bind(meter.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(row.0.unwrap_or(0.0))
    }

    async fn upsert_monthly(&self, row: &UsageMonthly) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO billing_usage_monthly
              (org_id, period_ym, meter, quantity, included_quantity, overage_quantity, amount_micros, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,NOW())
            ON CONFLICT (org_id, period_ym, meter) DO UPDATE SET
              quantity = EXCLUDED.quantity,
              included_quantity = EXCLUDED.included_quantity,
              overage_quantity = EXCLUDED.overage_quantity,
              amount_micros = EXCLUDED.amount_micros,
              updated_at = NOW()
            "#,
        )
        .bind(row.org_id.as_uuid())
        .bind(&row.period_ym)
        .bind(row.meter.as_str())
        .bind(row.quantity)
        .bind(row.included_quantity)
        .bind(row.overage_quantity)
        .bind(row.amount_micros)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(())
    }

    async fn list_monthly(&self, org_id: OrgId, period_ym: &str) -> Result<Vec<UsageMonthly>> {
        let rows: Vec<(String, f64, f64, f64, i64)> = sqlx::query_as(
            r#"SELECT meter, quantity, included_quantity, overage_quantity, amount_micros
               FROM billing_usage_monthly WHERE org_id = $1 AND period_ym = $2"#,
        )
        .bind(org_id.as_uuid())
        .bind(period_ym)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        let mut out = Vec::new();
        for (m, q, inc, over, amt) in rows {
            out.push(UsageMonthly {
                org_id,
                period_ym: period_ym.into(),
                meter: parse_meter(&m)?,
                quantity: q,
                included_quantity: inc,
                overage_quantity: over,
                amount_micros: amt,
            });
        }
        Ok(out)
    }

    async fn insert_managed(&self, r: &ManagedResource) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO billing_managed_resources
              (id, org_id, kind, name, sku, status, region, started_at, ended_at, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'{}'::jsonb)
            "#,
        )
        .bind(r.id)
        .bind(r.org_id.as_uuid())
        .bind(r.kind.as_str())
        .bind(&r.name)
        .bind(&r.sku)
        .bind(&r.status)
        .bind(&r.region)
        .bind(r.started_at)
        .bind(r.ended_at)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(())
    }

    async fn list_managed_active(&self, org_id: OrgId) -> Result<Vec<ManagedResource>> {
        let rows: Vec<(Uuid, String, String, String, String, Option<String>, chrono::DateTime<Utc>, Option<chrono::DateTime<Utc>>)> =
            sqlx::query_as(
                r#"SELECT id, kind, name, sku, status, region, started_at, ended_at
                   FROM billing_managed_resources
                   WHERE org_id = $1 AND status IN ('provisioning','active','suspended')"#,
            )
            .bind(org_id.as_uuid())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| BillingError::Other(e.into()))?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, kind, name, sku, status, region, started, ended)| {
                let kind = ManagedKind::parse(&kind)?;
                Some(ManagedResource {
                    id,
                    org_id,
                    kind,
                    name,
                    sku,
                    status,
                    region,
                    started_at: started,
                    ended_at: ended,
                })
            })
            .collect())
    }

    async fn platform_sums(&self, period_ym: &str) -> Result<(i64, i64, u32, u32)> {
        let usage: (Option<i64>,) = sqlx::query_as(
            "SELECT COALESCE(SUM(amount_micros),0) FROM billing_usage_monthly WHERE period_ym = $1",
        )
        .bind(period_ym)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        let seats: (Option<i64>, Option<i64>) = sqlx::query_as(
            r#"SELECT COALESCE(SUM(seat_quantity),0), COALESCE(COUNT(*),0)
               FROM billing_accounts WHERE status IN ('active','trialing','past_due')"#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok((
            0, // seat revenue computed in service from catalog
            usage.0.unwrap_or(0),
            seats.1.unwrap_or(0) as u32,
            seats.0.unwrap_or(0) as u32,
        ))
    }

    async fn insert_expense(
        &self,
        id: Uuid,
        period_ym: &str,
        category: &str,
        amount_micros: i64,
        currency: &str,
        vendor: Option<&str>,
        description: Option<&str>,
        created_by: Option<Uuid>,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO billing_expenses
               (id, period_ym, category, amount_micros, currency, vendor, description, created_by, created_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,NOW())"#,
        )
        .bind(id)
        .bind(period_ym)
        .bind(category)
        .bind(amount_micros)
        .bind(currency)
        .bind(vendor)
        .bind(description)
        .bind(created_by)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(())
    }

    async fn sum_expenses(&self, period_ym: &str) -> Result<(i64, Vec<(String, i64)>)> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"SELECT category, COALESCE(SUM(amount_micros),0)::bigint
               FROM billing_expenses WHERE period_ym = $1 GROUP BY category"#,
        )
        .bind(period_ym)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        let total: i64 = rows.iter().map(|(_, a)| *a).sum();
        Ok((total, rows))
    }

    async fn tax_settings(&self) -> Result<(String, i32)> {
        let row: Option<(String, i32)> = sqlx::query_as(
            "SELECT tax_name, rate_bps FROM billing_tax_settings ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(row.unwrap_or_else(|| ("VAT".into(), 1500)))
    }

    async fn invoice_stats(&self, period_ym: &str) -> Result<(u32, i64)> {
        let row = sqlx::query_as::<_, (i64, i64)>(
            r#"SELECT COUNT(*)::bigint,
                      COALESCE(SUM(CASE WHEN status = 'paid' THEN total_micros ELSE 0 END),0)::bigint
               FROM billing_invoices WHERE period_ym = $1"#,
        )
        .bind(period_ym)
        .fetch_one(&self.pool)
        .await;
        match row {
            Ok((c, p)) => Ok((c as u32, p)),
            Err(_) => Ok((0, 0)),
        }
    }


    async fn upsert_invoice(
        &self,
        id: Uuid,
        org_id: OrgId,
        period_ym: &str,
        status: &str,
        seat_amount_micros: i64,
        usage_amount_micros: i64,
        tax_amount_micros: i64,
        total_micros: i64,
        stripe_invoice_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO billing_invoices (
                id, org_id, period_ym, status, seat_amount_micros, usage_amount_micros,
                tax_amount_micros, total_micros, currency, issued_at, stripe_invoice_id, created_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'usd',NOW(),$9,NOW())
            ON CONFLICT (org_id, period_ym) DO UPDATE SET
                status = EXCLUDED.status,
                seat_amount_micros = EXCLUDED.seat_amount_micros,
                usage_amount_micros = EXCLUDED.usage_amount_micros,
                tax_amount_micros = EXCLUDED.tax_amount_micros,
                total_micros = EXCLUDED.total_micros,
                stripe_invoice_id = COALESCE(EXCLUDED.stripe_invoice_id, billing_invoices.stripe_invoice_id),
                issued_at = COALESCE(billing_invoices.issued_at, NOW())
            "#,
        )
        .bind(id)
        .bind(org_id.as_uuid())
        .bind(period_ym)
        .bind(status)
        .bind(seat_amount_micros)
        .bind(usage_amount_micros)
        .bind(tax_amount_micros)
        .bind(total_micros)
        .bind(stripe_invoice_id)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(())
    }

    async fn mark_invoice_paid(
        &self,
        org_id: OrgId,
        period_ym: &str,
        cash_micros: i64,
        stripe_invoice_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE billing_invoices SET
                status = 'paid',
                cash_amount_micros = $3,
                paid_at = NOW(),
                stripe_invoice_id = COALESCE($4, stripe_invoice_id)
            WHERE org_id = $1 AND period_ym = $2
            "#,
        )
        .bind(org_id.as_uuid())
        .bind(period_ym)
        .bind(cash_micros)
        .bind(stripe_invoice_id)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        // If no invoice row yet, still ok — stripe event is source of cash truth
        Ok(())
    }

    async fn insert_stripe_event(
        &self,
        id: Uuid,
        stripe_event_id: &str,
        event_type: &str,
        customer: Option<&str>,
        org_id: Option<OrgId>,
        amount_micros: i64,
        currency: &str,
        status: Option<&str>,
        period_ym: &str,
        raw: serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO billing_stripe_events (
                id, stripe_event_id, event_type, stripe_customer_id, org_id,
                amount_micros, currency, status, period_ym, raw, created_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,NOW())
            ON CONFLICT (stripe_event_id) DO NOTHING
            "#,
        )
        .bind(id)
        .bind(stripe_event_id)
        .bind(event_type)
        .bind(customer)
        .bind(org_id.map(|o| *o.as_uuid()))
        .bind(amount_micros)
        .bind(currency)
        .bind(status)
        .bind(period_ym)
        .bind(raw)
        .execute(&self.pool)
        .await
        .map_err(|e| BillingError::Other(e.into()))?;
        Ok(())
    }

    async fn sum_stripe_cash(&self, period_ym: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            r#"SELECT COALESCE(SUM(amount_micros),0)::bigint FROM billing_stripe_events
               WHERE period_ym = $1 AND event_type IN ('invoice.paid', 'charge.succeeded')"#,
        )
        .bind(period_ym)
        .fetch_one(&self.pool)
        .await
        .unwrap_or((0,));
        Ok(row.0)
    }

}

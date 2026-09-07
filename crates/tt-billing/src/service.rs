use std::sync::Arc;

use chrono::{Datelike, Utc};
use tracing::info;
use tt_core::ids::OrgId;
use uuid::Uuid;

use crate::catalog::{Catalog, Meter, PlanId};
use crate::domain::{
    BillingAccount, ManagedKind, ManagedResource, OrgInvoicePreview, PlatformFinanceSnapshot,
    SsoEntitlement, UsageEvent, UsageMonthly,
};
use crate::error::{BillingError, Result};
use crate::store::BillingStore;

pub struct BillingService {
    store: Arc<dyn BillingStore>,
    catalog: Catalog,
}

impl BillingService {
    pub fn new(store: Arc<dyn BillingStore>) -> Self {
        Self {
            store,
            catalog: Catalog::default_list(),
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn period_ym_now() -> String {
        let n = Utc::now();
        format!("{:04}-{:02}", n.year(), n.month())
    }

    /// Statuses that block new compute and git push (clone still allowed).
    pub fn status_blocks_mutations(status: &str) -> bool {
        matches!(
            status,
            "suspended" | "canceled" | "cancelled" | "unpaid" | "past_due"
        )
    }

    pub async fn is_org_active(&self, org_id: OrgId) -> Result<bool> {
        match self.store.get_account(org_id).await? {
            None => Ok(true), // no account yet → treat as active (free trial path)
            Some(a) => Ok(!Self::status_blocks_mutations(&a.status)),
        }
    }

    pub async fn assert_org_active(&self, org_id: OrgId) -> Result<()> {
        if self.is_org_active(org_id).await? {
            Ok(())
        } else {
            Err(BillingError::Forbidden(
                "organization billing status blocks this action (suspended/canceled/unpaid)".into(),
            ))
        }
    }

    /// SSO Wall — enabling or using org SSO requires Business or Enterprise.
    pub async fn assert_sso_entitled(&self, org_id: OrgId) -> Result<()> {
        self.assert_org_active(org_id).await?;
        let plan = match self.store.get_account(org_id).await? {
            Some(a) => a.plan_id,
            None => PlanId::Free,
        };
        if plan.includes_sso() {
            Ok(())
        } else {
            Err(BillingError::Forbidden(
                "SSO is available on Business and Enterprise plans (SSO wall)".into(),
            ))
        }
    }

    pub async fn assert_scim_entitled(&self, org_id: OrgId) -> Result<()> {
        self.assert_org_active(org_id).await?;
        let plan = match self.store.get_account(org_id).await? {
            Some(a) => a.plan_id,
            None => PlanId::Free,
        };
        if plan.includes_scim() {
            Ok(())
        } else {
            Err(BillingError::Forbidden(
                "SCIM directory sync is available on the Enterprise plan".into(),
            ))
        }
    }

    pub async fn sso_entitlement(&self, org_id: OrgId) -> Result<SsoEntitlement> {
        let plan = match self.store.get_account(org_id).await? {
            Some(a) => a.plan_id,
            None => PlanId::Free,
        };
        Ok(SsoEntitlement {
            plan_id: plan,
            sso_allowed: plan.includes_sso(),
            scim_allowed: plan.includes_scim(),
            sso_enforcement_allowed: plan.includes_sso_enforcement(),
        })
    }

    pub async fn ensure_account(
&self, org_id: OrgId, plan: PlanId, seats: u32) -> Result<BillingAccount> {
        if let Some(a) = self.store.get_account(org_id).await? {
            return Ok(a);
        }
        let acct = BillingAccount {
            org_id,
            plan_id: plan,
            stripe_customer_id: None,
            stripe_subscription_id: None,
            currency: "usd".into(),
            status: "active".into(),
            seat_quantity: seats.max(1),
            current_period_start: Some(Utc::now()),
            current_period_end: None,
        };
        self.store.upsert_account(&acct).await?;
        Ok(acct)
    }

    pub async fn get_account(&self, org_id: OrgId) -> Result<BillingAccount> {
        self.store
            .get_account(org_id)
            .await?
            .ok_or(BillingError::NotFound)
    }

    /// Record metered usage. Never folds into seat count.
    pub async fn record_usage(
        &self,
        org_id: OrgId,
        meter: Meter,
        quantity: f64,
        resource_id: Option<String>,
    ) -> Result<UsageEvent> {
        if quantity < 0.0 || !quantity.is_finite() {
            return Err(BillingError::Invalid("quantity must be finite >= 0".into()));
        }
        let ev = UsageEvent {
            id: Uuid::now_v7(),
            org_id,
            meter,
            quantity,
            unit: meter.unit().into(),
            resource_id,
            period_ym: Self::period_ym_now(),
            recorded_at: Utc::now(),
        };
        self.store.record_usage(&ev).await?;
        Ok(ev)
    }

    /// Recompute monthly rollup lines for an org period.
    pub async fn recompute_month(&self, org_id: OrgId, period_ym: &str) -> Result<Vec<UsageMonthly>> {
        let acct = self
            .store
            .get_account(org_id)
            .await?
            .unwrap_or(BillingAccount {
                org_id,
                plan_id: PlanId::Free,
                stripe_customer_id: None,
                stripe_subscription_id: None,
                currency: "usd".into(),
                status: "active".into(),
                seat_quantity: 1,
                current_period_start: None,
                current_period_end: None,
            });
        let meters = [
            Meter::ComputeVcpuHours,
            Meter::ComputeMemoryGbHours,
            Meter::GitStorageGbMonth,
            Meter::GitTransferGb,
            Meter::MessageStorageGbMonth,
            Meter::MediaStorageGbMonth,
            Meter::ManagedPostgresHours,
            Meter::ManagedCacheHours,
            Meter::ManagedStoreGbMonth,
            Meter::ManagedChronosHours,
        ];
        let mut lines = Vec::new();
        for m in meters {
            let qty = self.store.sum_usage(org_id, period_ym, m).await?;
            if qty <= 0.0 {
                continue;
            }
            let included = self.catalog.included_for(acct.plan_id, m);
            let (over, amount) = self.catalog.overage_micros(acct.plan_id, m, qty);
            let row = UsageMonthly {
                org_id,
                period_ym: period_ym.into(),
                meter: m,
                quantity: qty,
                included_quantity: included,
                overage_quantity: over,
                amount_micros: amount,
            };
            self.store.upsert_monthly(&row).await?;
            lines.push(row);
        }
        Ok(lines)
    }

    pub async fn invoice_preview(&self, org_id: OrgId, period_ym: Option<&str>) -> Result<OrgInvoicePreview> {
        let period = period_ym
            .map(|s| s.to_string())
            .unwrap_or_else(Self::period_ym_now);
        let acct = self.ensure_account(org_id, PlanId::Team, 1).await?;
        let lines = self.recompute_month(org_id, &period).await?;
        let usage_amount: i64 = lines.iter().map(|l| l.amount_micros).sum();
        let seat_amount = self
            .catalog
            .seat_monthly_micros(acct.plan_id, acct.seat_quantity);
        Ok(OrgInvoicePreview {
            org_id,
            period_ym: period,
            plan_id: acct.plan_id,
            seat_quantity: acct.seat_quantity,
            seat_amount_micros: seat_amount,
            usage_amount_micros: usage_amount,
            total_micros: seat_amount + usage_amount,
            currency: acct.currency,
            usage_lines: lines,
        })
    }

    pub async fn register_managed(
        &self,
        org_id: OrgId,
        kind: ManagedKind,
        name: &str,
        sku: &str,
        region: Option<String>,
    ) -> Result<ManagedResource> {
        let r = ManagedResource {
            id: Uuid::now_v7(),
            org_id,
            kind,
            name: name.trim().into(),
            sku: sku.into(),
            status: "active".into(),
            region,
            started_at: Utc::now(),
            ended_at: None,
        };
        if r.name.is_empty() {
            return Err(BillingError::Invalid("name required".into()));
        }
        self.store.insert_managed(&r).await?;
        info!(org = %org_id, kind = kind.as_str(), sku = %r.sku, "managed resource registered for billing");
        Ok(r)
    }

    /// Hourly tick: bill active managed instance-hours (call from worker).
    pub async fn tick_managed_hours(&self, org_id: OrgId, hours: f64) -> Result<()> {
        let active = self.store.list_managed_active(org_id).await?;
        for r in active {
            if r.status != "active" {
                continue;
            }
            let meter = r.kind.meter();
            // Object capacity is GB-month — not hourly
            if r.kind.is_storage_capacity() {
                continue;
            }
            self.record_usage(org_id, meter, hours, Some(r.id.to_string()))
                .await?;
        }
        Ok(())
    }

    pub async fn platform_finance(&self, period_ym: &str) -> Result<PlatformFinanceSnapshot> {
        let (_seat_placeholder, usage_rev, active_orgs, active_seats) =
            self.store.platform_sums(period_ym).await?;
        let seat_rev = self
            .catalog
            .seat_monthly_micros(PlanId::Team, active_seats);
        let total_rev = seat_rev + usage_rev;
        let mrr = total_rev; // period is monthly; refine with Stripe MRR later
        let arr = mrr.saturating_mul(12);
        let (expenses, by_cat) = self.store.sum_expenses(period_ym).await.unwrap_or((0, vec![]));
        let margin = total_rev - expenses;
        let margin_bps = if total_rev > 0 {
            (margin.saturating_mul(10_000)) / total_rev
        } else {
            0
        };
        let (tax_name, tax_bps) = self.store.tax_settings().await.unwrap_or(("VAT".into(), 1500));
        let est_tax = (total_rev.saturating_mul(tax_bps as i64)) / 10_000;
        let (inv_count, inv_paid) = self.store.invoice_stats(period_ym).await.unwrap_or((0, 0));
        let stripe_cash = self.store.sum_stripe_cash(period_ym).await.unwrap_or(0);
        Ok(PlatformFinanceSnapshot {
            period_ym: period_ym.into(),
            seat_revenue_micros: seat_rev,
            usage_revenue_micros: usage_rev,
            total_revenue_micros: total_rev,
            mrr_micros: mrr,
            arr_micros: arr,
            expenses_micros: expenses,
            expenses_by_category: by_cat,
            gross_margin_micros: margin,
            gross_margin_bps: margin_bps,
            tax_name,
            tax_rate_bps: tax_bps,
            estimated_tax_micros: est_tax,
            active_orgs,
            active_seats,
            invoice_count: inv_count,
            invoice_paid_micros: inv_paid,
            stripe_cash_micros: stripe_cash,
            currency: "usd".into(),
        })
    }

    pub async fn issue_invoice(
        &self,
        org_id: OrgId,
        period_ym: Option<&str>,
    ) -> Result<crate::domain::OrgInvoicePreview> {
        let period = period_ym
            .map(|s| s.to_string())
            .unwrap_or_else(Self::period_ym_now);
        let preview = self.invoice_preview(org_id, Some(&period)).await?;
        let (tax_name, tax_bps) = self.store.tax_settings().await.unwrap_or(("VAT".into(), 1500));
        let tax = (preview.total_micros.saturating_mul(tax_bps as i64)) / 10_000;
        let total = preview.total_micros + tax;
        let id = Uuid::now_v7();
        self.store
            .upsert_invoice(
                id,
                org_id,
                &period,
                "open",
                preview.seat_amount_micros,
                preview.usage_amount_micros,
                tax,
                total,
                None,
            )
            .await?;
        let _ = (tax_name, id);
        Ok(OrgInvoicePreview {
            org_id: preview.org_id,
            period_ym: preview.period_ym,
            plan_id: preview.plan_id,
            seat_quantity: preview.seat_quantity,
            seat_amount_micros: preview.seat_amount_micros,
            usage_lines: preview.usage_lines,
            usage_amount_micros: preview.usage_amount_micros,
            total_micros: total,
            currency: preview.currency,
        })
    }

    pub async fn mark_invoice_paid(
        &self,
        org_id: OrgId,
        period_ym: &str,
        cash_micros: i64,
        stripe_invoice_id: Option<&str>,
    ) -> Result<()> {
        self.store
            .mark_invoice_paid(org_id, period_ym, cash_micros, stripe_invoice_id)
            .await
    }

    pub async fn record_stripe_event(
        &self,
        stripe_event_id: &str,
        event_type: &str,
        customer: Option<&str>,
        amount_micros: i64,
        currency: &str,
        status: Option<&str>,
        raw: serde_json::Value,
    ) -> Result<Option<OrgId>> {
        let org_id = if let Some(c) = customer {
            self.store
                .get_account_by_stripe_customer(c)
                .await?
                .map(|a| a.org_id)
        } else {
            None
        };
        let period = Self::period_ym_now();
        self.store
            .insert_stripe_event(
                Uuid::now_v7(),
                stripe_event_id,
                event_type,
                customer,
                org_id,
                amount_micros,
                currency,
                status,
                &period,
                raw,
            )
            .await?;
        // Reconcile: paid invoices → mark local invoice + cash
        if event_type == "invoice.paid" || event_type == "charge.succeeded" {
            if let Some(oid) = org_id {
                let _ = self
                    .mark_invoice_paid(oid, &period, amount_micros, Some(stripe_event_id))
                    .await;
            }
        }
        if event_type == "invoice.payment_failed" {
            if let Some(oid) = org_id {
                let _ = self.set_plan(oid, PlanId::Team, 1, "past_due").await;
            }
        }
        Ok(org_id)
    }

    pub async fn stripe_cash_total(&self, period_ym: &str) -> Result<i64> {
        self.store.sum_stripe_cash(period_ym).await
    }

    pub async fn record_expense(

        &self,
        period_ym: &str,
        category: &str,
        amount_micros: i64,
        vendor: Option<String>,
        description: Option<String>,
        created_by: Option<Uuid>,
    ) -> Result<Uuid> {
        let allowed = [
            "metal", "power", "network", "storage", "software", "support", "tax", "other",
        ];
        if !allowed.contains(&category) {
            return Err(BillingError::Invalid(format!(
                "category must be one of {allowed:?}"
            )));
        }
        if amount_micros < 0 {
            return Err(BillingError::Invalid("amount must be >= 0".into()));
        }
        let id = Uuid::now_v7();
        self.store
            .insert_expense(
                id,
                period_ym,
                category,
                amount_micros,
                "usd",
                vendor.as_deref(),
                description.as_deref(),
                created_by,
            )
            .await?;
        Ok(id)
    }

    /// Emit compute vCPU-hours for a stopped instance.
    pub async fn emit_compute_hours(
        &self,
        org_id: OrgId,
        vcpu: u32,
        hours: f64,
        resource_id: Option<String>,
    ) -> Result<()> {
        let q = (vcpu as f64) * hours.max(0.0);
        if q <= 0.0 {
            return Ok(());
        }
        self.record_usage(org_id, Meter::ComputeVcpuHours, q, resource_id)
            .await?;
        Ok(())
    }

    /// Emit media storage GB (one-shot put → GB-month approximation for current period).
    pub async fn emit_media_bytes(&self, org_id: OrgId, bytes: u64, resource_id: Option<String>) -> Result<()> {
        let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        if gb <= 0.0 {
            return Ok(());
        }
        self.record_usage(org_id, Meter::MediaStorageGbMonth, gb, resource_id)
            .await?;
        Ok(())
    }

    pub async fn emit_git_storage_gb(&self, org_id: OrgId, gb: f64, resource_id: Option<String>) -> Result<()> {
        if gb <= 0.0 {
            return Ok(());
        }
        self.record_usage(org_id, Meter::GitStorageGbMonth, gb, resource_id)
            .await?;
        Ok(())
    }

    pub async fn set_plan(
        &self,
        org_id: OrgId,
        plan: PlanId,
        seats: u32,
        status: &str,
    ) -> Result<BillingAccount> {
        let mut acct = self
            .store
            .get_account(org_id)
            .await?
            .unwrap_or(BillingAccount {
                org_id,
                plan_id: plan,
                stripe_customer_id: None,
                stripe_subscription_id: None,
                currency: "usd".into(),
                status: status.into(),
                seat_quantity: seats.max(1),
                current_period_start: Some(chrono::Utc::now()),
                current_period_end: None,
            });
        acct.plan_id = plan;
        acct.seat_quantity = seats.max(1);
        acct.status = status.into();
        self.store.upsert_account(&acct).await?;
        Ok(acct)
    }

    pub async fn apply_stripe_seats(

        &self,
        customer: &str,
        sub_id: Option<&str>,
        seats: u32,
        status: &str,
        plan: PlanId,
    ) -> Result<Option<BillingAccount>> {
        let Some(mut acct) = self.store.get_account_by_stripe_customer(customer).await? else {
            return Ok(None);
        };
        acct.seat_quantity = seats.max(1);
        acct.status = status.into();
        acct.plan_id = plan;
        if let Some(s) = sub_id {
            acct.stripe_subscription_id = Some(s.into());
        }
        self.store.upsert_account(&acct).await?;
        Ok(Some(acct))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_blocks_mutations_table() {
        assert!(BillingService::status_blocks_mutations("suspended"));
        assert!(BillingService::status_blocks_mutations("canceled"));
        assert!(BillingService::status_blocks_mutations("unpaid"));
        assert!(BillingService::status_blocks_mutations("past_due"));
        assert!(!BillingService::status_blocks_mutations("active"));
        assert!(!BillingService::status_blocks_mutations("trialing"));
    }
}

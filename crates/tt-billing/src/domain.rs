use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::catalog::{Meter, PlanId};
use tt_core::ids::OrgId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingAccount {
    pub org_id: OrgId,
    pub plan_id: PlanId,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub currency: String,
    pub status: String,
    pub seat_quantity: u32,
    pub current_period_start: Option<DateTime<Utc>>,
    pub current_period_end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub id: Uuid,
    pub org_id: OrgId,
    pub meter: Meter,
    pub quantity: f64,
    pub unit: String,
    pub resource_id: Option<String>,
    pub period_ym: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageMonthly {
    pub org_id: OrgId,
    pub period_ym: String,
    pub meter: Meter,
    pub quantity: f64,
    pub included_quantity: f64,
    pub overage_quantity: f64,
    pub amount_micros: i64,
}


/// Managed data products on TT Cloud.
///
/// One brand per engine: **Turing***. UI shows what powers it so customers
/// know TT runs the same technology (Ember / RustFS / GreptimeDB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedKind {
    /// Unbranded PostgreSQL
    Postgres,
    /// TuringCache — powered by Ember (Kacy Fortner)
    TuringCache,
    /// TuringStore — powered by RustFS
    TuringStore,
    /// TuringChronos — powered by GreptimeDB
    TuringChronos,
}

impl ManagedKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::TuringCache => "turing_cache",
            Self::TuringStore => "turing_store",
            Self::TuringChronos => "turing_chronos",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "postgres" => Some(Self::Postgres),
            "turing_cache" | "emberdb" | "ember" | "ember_vanilla" => Some(Self::TuringCache),
            "turing_store" | "rustfs" | "rustfs_vanilla" => Some(Self::TuringStore),
            "turing_chronos" | "greptimedb" | "greptime" | "greptime_vanilla" => {
                Some(Self::TuringChronos)
            }
            _ => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::TuringCache => "TuringCache",
            Self::TuringStore => "TuringStore",
            Self::TuringChronos => "TuringChronos",
        }
    }

    /// Short "powered by" line for provisioning UI.
    pub fn powered_by(self) -> Option<&'static str> {
        match self {
            Self::Postgres => None,
            Self::TuringCache => Some("Ember"),
            Self::TuringStore => Some("RustFS"),
            Self::TuringChronos => Some("GreptimeDB"),
        }
    }

    /// Full attribution for credits strip / about panel.
    pub fn attribution(self) -> Option<&'static str> {
        match self {
            Self::Postgres => None,
            Self::TuringCache => Some("Powered by Ember by Kacy Fortner (@kacyf)"),
            Self::TuringStore => Some("Powered by RustFS — with thanks to the RustFS team"),
            Self::TuringChronos => {
                Some("Powered by GreptimeDB — with thanks to the Greptime team")
            }
        }
    }

    pub fn meter(self) -> Meter {
        match self {
            Self::Postgres => Meter::ManagedPostgresHours,
            Self::TuringCache => Meter::ManagedCacheHours,
            Self::TuringStore => Meter::ManagedStoreGbMonth,
            Self::TuringChronos => Meter::ManagedChronosHours,
        }
    }

    pub fn is_storage_capacity(self) -> bool {
        matches!(self, Self::TuringStore)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedResource {
    pub id: Uuid,
    pub org_id: OrgId,
    pub kind: ManagedKind,
    pub name: String,
    pub sku: String,
    pub status: String,
    pub region: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgInvoicePreview {
    pub org_id: OrgId,
    pub period_ym: String,
    pub plan_id: PlanId,
    pub seat_quantity: u32,
    pub seat_amount_micros: i64,
    pub usage_lines: Vec<UsageMonthly>,
    pub usage_amount_micros: i64,
    pub total_micros: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformFinanceSnapshot {
    pub period_ym: String,
    /// Recognized seat revenue (catalog × active seats — refine with Stripe later)
    pub seat_revenue_micros: i64,
    pub usage_revenue_micros: i64,
    pub total_revenue_micros: i64,
    /// Annualized run-rate from current MRR
    pub mrr_micros: i64,
    pub arr_micros: i64,
    /// Platform COGS / opex booked in period
    pub expenses_micros: i64,
    pub expenses_by_category: Vec<(String, i64)>,
    /// total_revenue - expenses
    pub gross_margin_micros: i64,
    pub gross_margin_bps: i64,
    pub tax_name: String,
    pub tax_rate_bps: i32,
    /// Estimated tax on revenue if exclusive
    pub estimated_tax_micros: i64,
    pub active_orgs: u32,
    pub active_seats: u32,
    pub invoice_count: u32,
    pub invoice_paid_micros: i64,
    /// Cash collected via Stripe webhooks this period
    pub stripe_cash_micros: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpenseEntry {
    pub id: Uuid,
    pub period_ym: String,
    pub category: String,
    pub amount_micros: i64,
    pub currency: String,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powered_by_and_attribution() {
        assert_eq!(ManagedKind::TuringCache.powered_by(), Some("Ember"));
        assert_eq!(ManagedKind::TuringStore.powered_by(), Some("RustFS"));
        assert_eq!(ManagedKind::TuringChronos.powered_by(), Some("GreptimeDB"));
        assert!(ManagedKind::TuringCache.attribution().unwrap().contains("Kacy"));
        assert!(ManagedKind::Postgres.attribution().is_none());
    }

    #[test]
    fn aliases_map_to_turing_only() {
        assert_eq!(ManagedKind::parse("ember"), Some(ManagedKind::TuringCache));
        assert_eq!(ManagedKind::parse("ember_vanilla"), Some(ManagedKind::TuringCache));
        assert_eq!(ManagedKind::parse("rustfs"), Some(ManagedKind::TuringStore));
        assert_eq!(ManagedKind::parse("greptimedb"), Some(ManagedKind::TuringChronos));
    }
}

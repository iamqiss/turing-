//! List prices — undercut GH+Slack+cloud *sum*, stay above TT metal cost.
//!
//! Amounts are **USD micros** (1_000_000 = $1.00).
//! Tune via `billing_price_overrides` without redeploying when needed.
//!
//! Design rules:
//! 1. Seats fund collaboration (chat, git, rooms, issues) — predictable ARR.
//! 2. Usage meters fund variable cost (compute, storage, managed data) — protects margin.
//! 3. Never bundle unlimited compute into seats (3 humans can outspend 50 chat-only).
//! 4. Included allowances on plans are *soft product value*, not blank checks.

use serde::{Deserialize, Serialize};

/// 1 USD = 1_000_000 micros
pub type Micros = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanId {
    Free,
    Team,
    Business,
    Enterprise,
}

impl PlanId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Team => "team",
            Self::Business => "business",
            Self::Enterprise => "enterprise",
        }
    }

    /// SSO wall: Business+ (OIDC/SAML). Free/Team stay passphrase/passkey.
    pub fn includes_sso(self) -> bool {
        matches!(self, Self::Business | Self::Enterprise)
    }

    /// SCIM / directory sync — Enterprise only.
    pub fn includes_scim(self) -> bool {
        matches!(self, Self::Enterprise)
    }

    /// Enforce SSO for domain members (IdP-required login).
    pub fn includes_sso_enforcement(self) -> bool {
        matches!(self, Self::Business | Self::Enterprise)
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "free" => Some(Self::Free),
            "team" => Some(Self::Team),
            "business" => Some(Self::Business),
            "enterprise" => Some(Self::Enterprise),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Meter {
    /// Firecracker / CH vCPU-hours (post-overcommit sellable capacity)
    ComputeVcpuHours,
    /// Memory GB-hours
    ComputeMemoryGbHours,
    /// Git bare repo storage
    GitStorageGbMonth,
    /// Git LFS / pack transfer egress
    GitTransferGb,
    /// Channel + room message text storage (compressed estimate)
    MessageStorageGbMonth,
    /// Media object bytes
    MediaStorageGbMonth,
    /// Managed Postgres instance-hours
    ManagedPostgresHours,
    /// TuringCache (Ember) instance-hours
    ManagedCacheHours,
    /// TuringStore (RustFS) capacity GB-month
    ManagedStoreGbMonth,
    /// TuringChronos (GreptimeDB) instance-hours
    ManagedChronosHours,
    /// Turing Tasks CI minutes
    TasksMinutes,
}

impl Meter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ComputeVcpuHours => "compute.vcpu_hours",
            Self::ComputeMemoryGbHours => "compute.memory_gb_hours",
            Self::GitStorageGbMonth => "git.storage_gb_month",
            Self::GitTransferGb => "git.transfer_gb",
            Self::MessageStorageGbMonth => "msg.storage_gb_month",
            Self::MediaStorageGbMonth => "media.storage_gb_month",
            Self::ManagedPostgresHours => "managed.postgres_hours",
            Self::ManagedCacheHours => "managed.cache_hours",
            Self::ManagedStoreGbMonth => "managed.store_gb_month",
            Self::ManagedChronosHours => "managed.chronos_hours",
            Self::TasksMinutes => "tasks.minutes",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::ComputeVcpuHours => "vcpu_hour",
            Self::ComputeMemoryGbHours => "gb_hour",
            Self::GitStorageGbMonth
            | Self::MessageStorageGbMonth
            | Self::MediaStorageGbMonth
            | Self::ManagedStoreGbMonth => "gb_month",
            Self::GitTransferGb => "gb",
            Self::ManagedPostgresHours
            | Self::ManagedCacheHours
            | Self::ManagedChronosHours => "instance_hour",
            Self::TasksMinutes => "minute",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "compute.vcpu_hours" => Some(Self::ComputeVcpuHours),
            "compute.memory_gb_hours" => Some(Self::ComputeMemoryGbHours),
            "git.storage_gb_month" => Some(Self::GitStorageGbMonth),
            "git.transfer_gb" => Some(Self::GitTransferGb),
            "msg.storage_gb_month" => Some(Self::MessageStorageGbMonth),
            "media.storage_gb_month" => Some(Self::MediaStorageGbMonth),
            "managed.postgres_hours" => Some(Self::ManagedPostgresHours),
            "managed.cache_hours" | "managed.ember_hours" => Some(Self::ManagedCacheHours),
            "managed.store_gb_month" | "managed.rustfs_gb_month" => Some(Self::ManagedStoreGbMonth),
            "managed.chronos_hours" | "managed.greptime_hours" => Some(Self::ManagedChronosHours),
            "tasks.minutes" => Some(Self::TasksMinutes),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: PlanId,
    pub name: String,
    /// Per active human seat / month
    pub seat_price_micros: Micros,
    pub included_seats: u32,
    /// Soft included usage per month (then overage)
    pub included: Vec<(Meter, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub plans: Vec<Plan>,
    /// Overage / pure usage list prices
    pub meter_price_micros: Vec<(Meter, Micros)>,
}

impl Catalog {
    /// Default TT catalog — competitive vs sum of counterparts, margin-aware on metal.
    ///
    /// Rough anchors (2026):
    /// - GitHub Team ~$4/seat + Slack Pro ~$8.75 → ~$13 bundle; TT Team **$10/seat** all-in collab
    /// - Business undercuts GH Enterprise-ish + Slack Business+ stacks
    /// - Compute: metal ~R1–2k/mo (~$55–110) for 32c; 4:1 overcommit → sellable hours priced
    ///   so blended margin stays healthy after power/network/support
    pub fn default_list() -> Self {
        Self {
            plans: vec![
                Plan {
                    id: PlanId::Free,
                    name: "Free".into(),
                    seat_price_micros: 0,
                    included_seats: 3,
                    included: vec![
                        (Meter::GitStorageGbMonth, 1.0),
                        (Meter::MediaStorageGbMonth, 1.0),
                        (Meter::ComputeVcpuHours, 20.0),
                        (Meter::MessageStorageGbMonth, 0.5),
                    ],
                },
                Plan {
                    id: PlanId::Team,
                    name: "Team".into(),
                    // $10 / seat / mo
                    seat_price_micros: 10_000_000,
                    included_seats: 0, // all seats paid
                    included: vec![
                        (Meter::GitStorageGbMonth, 5.0),
                        (Meter::MediaStorageGbMonth, 10.0),
                        (Meter::ComputeVcpuHours, 100.0),
                        (Meter::GitTransferGb, 50.0),
                        (Meter::MessageStorageGbMonth, 2.0),
                    ],
                },
                Plan {
                    id: PlanId::Business,
                    name: "Business".into(),
                    // $18 / seat / mo
                    seat_price_micros: 18_000_000,
                    included_seats: 0,
                    included: vec![
                        (Meter::GitStorageGbMonth, 25.0),
                        (Meter::MediaStorageGbMonth, 50.0),
                        (Meter::ComputeVcpuHours, 400.0),
                        (Meter::GitTransferGb, 200.0),
                        (Meter::MessageStorageGbMonth, 10.0),
                    ],
                },
                Plan {
                    id: PlanId::Enterprise,
                    name: "Enterprise".into(),
                    // list anchor; real deals negotiated
                    seat_price_micros: 30_000_000,
                    included_seats: 0,
                    included: vec![
                        (Meter::GitStorageGbMonth, 100.0),
                        (Meter::MediaStorageGbMonth, 200.0),
                        (Meter::ComputeVcpuHours, 2_000.0),
                        (Meter::GitTransferGb, 1_000.0),
                        (Meter::MessageStorageGbMonth, 50.0),
                    ],
                },
            ],
            meter_price_micros: vec![
                // ~$0.04 / vCPU-hour list — under public cloud burst, above amortized metal
                (Meter::ComputeVcpuHours, 40_000),
                (Meter::ComputeMemoryGbHours, 5_000),
                // git storage ~$0.015/GB-mo
                (Meter::GitStorageGbMonth, 15_000),
                (Meter::GitTransferGb, 80_000), // $0.08/GB egress-ish
                (Meter::MessageStorageGbMonth, 20_000),
                (Meter::MediaStorageGbMonth, 15_000),
                // managed data — undercut RDS/Atlas class, cover HA ops
                (Meter::ManagedPostgresHours, 50_000), // $0.05/hr small SKU baseline
                (Meter::ManagedCacheHours, 35_000), // under managed Redis-class list
                (Meter::ManagedStoreGbMonth, 12_000),
                (Meter::ManagedChronosHours, 45_000),
            ],
        }
    }

    pub fn plan(&self, id: PlanId) -> Option<&Plan> {
        self.plans.iter().find(|p| p.id == id)
    }

    pub fn meter_price(&self, m: Meter) -> Micros {
        self.meter_price_micros
            .iter()
            .find(|(k, _)| *k == m)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    }

    pub fn included_for(&self, plan: PlanId, m: Meter) -> f64 {
        self.plan(plan)
            .and_then(|p| p.included.iter().find(|(k, _)| *k == m).map(|(_, q)| *q))
            .unwrap_or(0.0)
    }

    /// Seat ARR for quantity on a plan (monthly micros).
    pub fn seat_monthly_micros(&self, plan: PlanId, seats: u32) -> Micros {
        let Some(p) = self.plan(plan) else {
            return 0;
        };
        let billable = seats.saturating_sub(p.included_seats);
        p.seat_price_micros.saturating_mul(billable as i64)
    }

    /// Overage charge given raw usage quantity this period.
    pub fn overage_micros(&self, plan: PlanId, m: Meter, quantity: f64) -> (f64, Micros) {
        let included = self.included_for(plan, m);
        let over = (quantity - included).max(0.0);
        let price = self.meter_price(m);
        let amount = (over * price as f64).round() as Micros;
        (over, amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_undercuts_naive_gh_slack_sum() {
        let c = Catalog::default_list();
        // 10 seats team = $100/mo vs ~$127.50 GH Team+Slack Pro
        let seats = c.seat_monthly_micros(PlanId::Team, 10);
        assert_eq!(seats, 100_000_000);
    }

    #[test]
    fn compute_overage_not_swallowed_by_seats() {
        let c = Catalog::default_list();
        // 3-seat team burns 500 vCPU-h → 400 over included 100
        let (over, amount) = c.overage_micros(PlanId::Team, Meter::ComputeVcpuHours, 500.0);
        assert!((over - 400.0).abs() < f64::EPSILON);
        assert_eq!(amount, 400 * 40_000);
    }

    #[test]
    fn free_has_no_seat_charge() {
        let c = Catalog::default_list();
        assert_eq!(c.seat_monthly_micros(PlanId::Free, 3), 0);
    }
}

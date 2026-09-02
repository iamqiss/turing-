-- Dual-track billing: seats (collaboration) + usage meters (compute / storage / managed data)

CREATE TABLE IF NOT EXISTS billing_accounts (
    org_id              UUID PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    plan_id             TEXT NOT NULL DEFAULT 'team',
    stripe_customer_id  TEXT,
    stripe_subscription_id TEXT,
    currency            TEXT NOT NULL DEFAULT 'usd',
    status              TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('trialing', 'active', 'past_due', 'canceled', 'unpaid')),
    seat_quantity       INT NOT NULL DEFAULT 1 CHECK (seat_quantity >= 0),
    current_period_start TIMESTAMPTZ,
    current_period_end   TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS billing_accounts_stripe_idx
    ON billing_accounts (stripe_customer_id) WHERE stripe_customer_id IS NOT NULL;

-- Immutable catalog snapshots can live in code; this table overrides list prices per env if needed
CREATE TABLE IF NOT EXISTS billing_price_overrides (
    meter_or_plan       TEXT PRIMARY KEY,
    unit_amount_micros  BIGINT NOT NULL, -- USD micros (1e-6 dollar)
    note                TEXT,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Append-only usage events (source of truth for invoices)
CREATE TABLE IF NOT EXISTS billing_usage_events (
    id                  UUID PRIMARY KEY,
    org_id              UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    meter               TEXT NOT NULL,
    quantity            DOUBLE PRECISION NOT NULL CHECK (quantity >= 0),
    unit                TEXT NOT NULL,
    resource_id         TEXT,
    period_ym           TEXT NOT NULL, -- YYYY-MM for fast rollups
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS billing_usage_org_period_idx
    ON billing_usage_events (org_id, period_ym, meter);
CREATE INDEX IF NOT EXISTS billing_usage_org_recorded_idx
    ON billing_usage_events (org_id, recorded_at DESC);

-- Materialized-ish monthly rollup (updated by aggregator job)
CREATE TABLE IF NOT EXISTS billing_usage_monthly (
    org_id              UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    period_ym           TEXT NOT NULL,
    meter               TEXT NOT NULL,
    quantity            DOUBLE PRECISION NOT NULL DEFAULT 0,
    included_quantity   DOUBLE PRECISION NOT NULL DEFAULT 0,
    overage_quantity    DOUBLE PRECISION NOT NULL DEFAULT 0,
    amount_micros       BIGINT NOT NULL DEFAULT 0,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, period_ym, meter)
);

-- Provisioned managed data products (bill while alive)
CREATE TABLE IF NOT EXISTS billing_managed_resources (
    id                  UUID PRIMARY KEY,
    org_id              UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL
                        CHECK (kind IN ('postgres', 'emberdb', 'rustfs', 'greptimedb')),
    name                TEXT NOT NULL,
    sku                 TEXT NOT NULL, -- e.g. pg.shared.1g, ember.ha.small
    status              TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('provisioning', 'active', 'suspended', 'deleted')),
    region              TEXT,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at            TIMESTAMPTZ,
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS billing_managed_org_idx
    ON billing_managed_resources (org_id, status);

-- Platform finance snapshot (admin)
CREATE TABLE IF NOT EXISTS billing_ledger_entries (
    id                  UUID PRIMARY KEY,
    org_id              UUID REFERENCES orgs(id) ON DELETE SET NULL,
    entry_type          TEXT NOT NULL, -- seat_charge | usage_charge | credit | refund | cost_internal
    amount_micros       BIGINT NOT NULL,
    currency            TEXT NOT NULL DEFAULT 'usd',
    description         TEXT,
    period_ym           TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS billing_ledger_created_idx ON billing_ledger_entries (created_at DESC);
CREATE INDEX IF NOT EXISTS billing_ledger_org_idx ON billing_ledger_entries (org_id, created_at DESC);

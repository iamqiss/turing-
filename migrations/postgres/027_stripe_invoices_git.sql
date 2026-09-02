-- Stripe cash events + formal invoices already partially in 026; extend + git size cache

CREATE TABLE IF NOT EXISTS billing_stripe_events (
    id                  UUID PRIMARY KEY,
    stripe_event_id     TEXT NOT NULL UNIQUE,
    event_type          TEXT NOT NULL,
    stripe_customer_id  TEXT,
    org_id              UUID REFERENCES orgs(id) ON DELETE SET NULL,
    amount_micros       BIGINT NOT NULL DEFAULT 0,
    currency            TEXT NOT NULL DEFAULT 'usd',
    status              TEXT, -- paid | failed | open | ...
    period_ym           TEXT,
    raw                 JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS billing_stripe_events_period_idx
    ON billing_stripe_events (period_ym, event_type);
CREATE INDEX IF NOT EXISTS billing_stripe_events_customer_idx
    ON billing_stripe_events (stripe_customer_id);

-- Cash collected this period (from Stripe), separate from catalog MRR estimate
ALTER TABLE billing_invoices
    ADD COLUMN IF NOT EXISTS stripe_invoice_id TEXT,
    ADD COLUMN IF NOT EXISTS stripe_payment_intent TEXT,
    ADD COLUMN IF NOT EXISTS cash_amount_micros BIGINT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS git_storage_snapshots (
    org_id              UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    period_ym           TEXT NOT NULL,
    bytes_total         BIGINT NOT NULL DEFAULT 0,
    repo_count          INT NOT NULL DEFAULT 0,
    scanned_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, period_ym)
);

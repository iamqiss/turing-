-- Platform expenses (COGS / opex) + tax settings + invoice headers

CREATE TABLE IF NOT EXISTS billing_expenses (
    id                  UUID PRIMARY KEY,
    period_ym           TEXT NOT NULL,
    category            TEXT NOT NULL,
    -- metal | power | network | storage | software | support | tax | other
    amount_micros       BIGINT NOT NULL,
    currency            TEXT NOT NULL DEFAULT 'usd',
    vendor              TEXT,
    description         TEXT,
    created_by          UUID REFERENCES principals(id),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS billing_expenses_period_idx ON billing_expenses (period_ym, category);

CREATE TABLE IF NOT EXISTS billing_tax_settings (
    id                  UUID PRIMARY KEY,
    jurisdiction        TEXT NOT NULL DEFAULT 'ZA',
    tax_name            TEXT NOT NULL DEFAULT 'VAT',
    rate_bps            INT NOT NULL DEFAULT 1500, -- 15.00% = 1500 bps
    inclusive           BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO billing_tax_settings (id, jurisdiction, tax_name, rate_bps, inclusive)
SELECT '01900000-0000-7000-8000-0000000000aa'::uuid, 'ZA', 'VAT', 1500, false
WHERE NOT EXISTS (SELECT 1 FROM billing_tax_settings LIMIT 1);

CREATE TABLE IF NOT EXISTS billing_invoices (
    id                  UUID PRIMARY KEY,
    org_id              UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    period_ym           TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'draft'
                        CHECK (status IN ('draft', 'open', 'paid', 'void', 'uncollectible')),
    seat_amount_micros  BIGINT NOT NULL DEFAULT 0,
    usage_amount_micros BIGINT NOT NULL DEFAULT 0,
    tax_amount_micros   BIGINT NOT NULL DEFAULT 0,
    total_micros        BIGINT NOT NULL DEFAULT 0,
    currency            TEXT NOT NULL DEFAULT 'usd',
    issued_at           TIMESTAMPTZ,
    due_at              TIMESTAMPTZ,
    paid_at             TIMESTAMPTZ,
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (org_id, period_ym)
);

CREATE INDEX IF NOT EXISTS billing_invoices_period_idx ON billing_invoices (period_ym, status);

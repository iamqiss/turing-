-- Break Glass recovery: time-lock + trustees (optional, user-enabled)

CREATE TABLE IF NOT EXISTS break_glass_settings (
    principal_id              UUID PRIMARY KEY REFERENCES principals(id) ON DELETE CASCADE,
    time_lock_enabled         BOOLEAN NOT NULL DEFAULT FALSE,
    time_lock_delay_secs      BIGINT  NOT NULL DEFAULT 604800, -- 7 days
    trustees_enabled          BOOLEAN NOT NULL DEFAULT FALSE,
    trustee_threshold         INT     NOT NULL DEFAULT 3,
    -- Optional contact channels for time-lock broadcast (ops / email / SMS hooks)
    notify_email              TEXT,
    notify_phone              TEXT,
    created_at                TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER break_glass_settings_updated_at
    BEFORE UPDATE ON break_glass_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Trustees: public identity material only.
-- trustee_pubkey is an opaque verification key (hex/base64); binding_proof ties it
-- to a ceremony without storing a social graph in the clear if the client so chooses.
-- If trustee_principal_id is set, that human can approve from their TT session.
CREATE TABLE IF NOT EXISTS break_glass_trustees (
    id                   UUID PRIMARY KEY,
    principal_id         UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    label_commitment     TEXT NOT NULL, -- client-side label hash; server never needs clear name
    trustee_pubkey       TEXT NOT NULL,
    trustee_principal_id UUID REFERENCES principals(id) ON DELETE SET NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (principal_id, trustee_pubkey)
);

CREATE INDEX break_glass_trustees_principal_idx ON break_glass_trustees (principal_id);
CREATE INDEX break_glass_trustees_by_trustee_idx ON break_glass_trustees (trustee_principal_id)
    WHERE trustee_principal_id IS NOT NULL;

-- Active / historical recovery events
CREATE TABLE IF NOT EXISTS break_glass_events (
    id                UUID PRIMARY KEY,
    principal_id      UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    method            TEXT NOT NULL CHECK (method IN ('time_lock', 'trustees')),
    status            TEXT NOT NULL CHECK (status IN (
        'pending', 'cancelled', 'ready', 'completed', 'expired'
    )),
    -- time-lock
    unlock_at         TIMESTAMPTZ,
    cancel_token_hash TEXT,
    -- trustees
    threshold         INT,
    -- completion
    recovery_token_hash TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    cancelled_at      TIMESTAMPTZ,
    completed_at      TIMESTAMPTZ
);

CREATE INDEX break_glass_events_principal_idx ON break_glass_events (principal_id, status);
CREATE INDEX break_glass_events_unlock_idx ON break_glass_events (status, unlock_at)
    WHERE method = 'time_lock';

CREATE TRIGGER break_glass_events_updated_at
    BEFORE UPDATE ON break_glass_events
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE IF NOT EXISTS break_glass_approvals (
    event_id     UUID NOT NULL REFERENCES break_glass_events(id) ON DELETE CASCADE,
    trustee_id   UUID NOT NULL REFERENCES break_glass_trustees(id) ON DELETE CASCADE,
    signature    TEXT NOT NULL,
    signed_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, trustee_id)
);

-- Outbox for "broadcast to devices / email / phone" (workers drain this)
CREATE TABLE IF NOT EXISTS break_glass_notifications (
    id           UUID PRIMARY KEY,
    event_id     UUID NOT NULL REFERENCES break_glass_events(id) ON DELETE CASCADE,
    channel      TEXT NOT NULL CHECK (channel IN ('session', 'email', 'phone', 'push')),
    target       TEXT NOT NULL,
    payload      JSONB NOT NULL DEFAULT '{}',
    sent_at      TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX break_glass_notifications_pending_idx
    ON break_glass_notifications (sent_at) WHERE sent_at IS NULL;

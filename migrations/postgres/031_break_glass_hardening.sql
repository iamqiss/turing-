-- Hardening: rate limits, single-use recovery claim, approval nonces

ALTER TABLE break_glass_events
    ADD COLUMN IF NOT EXISTS recovery_claimed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS trigger_ip_hash TEXT,
    ADD COLUMN IF NOT EXISTS approval_message TEXT;

CREATE TABLE IF NOT EXISTS break_glass_rate_limits (
    bucket_key   TEXT PRIMARY KEY,
    window_start TIMESTAMPTZ NOT NULL,
    hit_count    INT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS break_glass_events_pending_unlock
    ON break_glass_events (unlock_at)
    WHERE method = 'time_lock' AND status = 'pending';

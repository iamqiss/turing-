-- Superadmin / platform audit trail
CREATE TABLE IF NOT EXISTS admin_audit_log (
    id              UUID PRIMARY KEY,
    actor_id        UUID NOT NULL REFERENCES principals(id),
    action          TEXT NOT NULL,
    target_type     TEXT NOT NULL, -- org | principal | platform
    target_id       TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS admin_audit_created_idx ON admin_audit_log (created_at DESC);
CREATE INDEX IF NOT EXISTS admin_audit_actor_idx ON admin_audit_log (actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS admin_audit_target_idx ON admin_audit_log (target_type, target_id);

-- Encrypted secrets at rest + platform operators already partially in 014

ALTER TABLE compute_nodes
    ADD COLUMN IF NOT EXISTS agent_token_enc BYTEA;

ALTER TABLE deploy_tokens
    ADD COLUMN IF NOT EXISTS token_enc BYTEA;

CREATE TABLE IF NOT EXISTS platform_operators (
    principal_id    UUID PRIMARY KEY REFERENCES principals(id) ON DELETE CASCADE,
    note            TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

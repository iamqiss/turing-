-- Scarlett Letter ✉ — org/workspace secrets (encrypted at rest by control plane)

CREATE TABLE IF NOT EXISTS scarlett_secrets (
    id              UUID PRIMARY KEY,
    org_id          UUID REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id    UUID REFERENCES workspaces(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    ciphertext      BYTEA NOT NULL,
    created_by      UUID NOT NULL REFERENCES principals(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workspace_id, name)
);

CREATE INDEX IF NOT EXISTS scarlett_secrets_ws_idx ON scarlett_secrets (workspace_id);

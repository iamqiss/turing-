-- Short-lived deploy tokens for private git clone from tt-node
CREATE TABLE IF NOT EXISTS deploy_tokens (
    id              UUID PRIMARY KEY,
    org_id          UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    repository_id   UUID REFERENCES repositories(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principals(id),
    token_hash      TEXT NOT NULL,
    token_prefix    TEXT NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS deploy_tokens_prefix_idx ON deploy_tokens (token_prefix);
CREATE INDEX IF NOT EXISTS deploy_tokens_org_idx ON deploy_tokens (org_id);

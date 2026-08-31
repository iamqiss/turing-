-- Scarlet Letter ✉ — kills .env
-- Org / environment / repo scoped secrets with envelope encryption + audit

CREATE TABLE IF NOT EXISTS scarlet_secrets (
    id              UUID PRIMARY KEY,
    org_id          UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- Scope: exactly one of environment or repository may be set; both null = org-level
    environment     TEXT,                 -- dev | staging | prod | custom
    repository_id   UUID REFERENCES repositories(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,        -- e.g. DATABASE_URL
    -- Metadata Pearl / IDE may see (never the value)
    value_kind      TEXT NOT NULL DEFAULT 'string',  -- string | url | connection | token | json | binary
    description     TEXT,
    -- Envelope: DEK encrypted with org master key material (server KEK)
    wrapped_dek     BYTEA NOT NULL,
    ciphertext      BYTEA NOT NULL,      -- secret encrypted under DEK
    nonce           BYTEA NOT NULL,
    version         INT NOT NULL DEFAULT 1,
    created_by      UUID NOT NULL REFERENCES principals(id),
    updated_by      UUID NOT NULL REFERENCES principals(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_rotated_at TIMESTAMPTZ
);

-- Uniqueness per scope
CREATE UNIQUE INDEX IF NOT EXISTS scarlet_secrets_org_name_uidx
    ON scarlet_secrets (org_id, name)
    WHERE environment IS NULL AND repository_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS scarlet_secrets_env_name_uidx
    ON scarlet_secrets (org_id, environment, name)
    WHERE environment IS NOT NULL AND repository_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS scarlet_secrets_repo_name_uidx
    ON scarlet_secrets (repository_id, name)
    WHERE repository_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS scarlet_secrets_org_idx ON scarlet_secrets (org_id);

CREATE TABLE IF NOT EXISTS scarlet_access_audit (
    id              UUID PRIMARY KEY,
    secret_id       UUID NOT NULL REFERENCES scarlet_secrets(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principals(id),
    action          TEXT NOT NULL,  -- put | get | list | pull | rotate | delete | pearl_meta
    client_kind     TEXT NOT NULL DEFAULT 'api', -- api | cli | ide | pearl | ci
    ip_hint         TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS scarlet_access_audit_secret_idx
    ON scarlet_access_audit (secret_id, created_at DESC);
CREATE INDEX IF NOT EXISTS scarlet_access_audit_principal_idx
    ON scarlet_access_audit (principal_id, created_at DESC);

-- Optional: grant who may decrypt (beyond org role defaults)
-- Note: Postgres disallows expressions in PRIMARY KEY; use id + unique indexes.
CREATE TABLE IF NOT EXISTS scarlet_grants (
    id              UUID PRIMARY KEY,
    secret_id       UUID NOT NULL REFERENCES scarlet_secrets(id) ON DELETE CASCADE,
    principal_id    UUID REFERENCES principals(id) ON DELETE CASCADE,
    role            TEXT,  -- if set, any principal with this org role
    can_read        BOOLEAN NOT NULL DEFAULT TRUE,
    can_write       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (principal_id IS NOT NULL OR role IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS scarlet_grants_principal_uidx
    ON scarlet_grants (secret_id, principal_id)
    WHERE principal_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS scarlet_grants_role_uidx
    ON scarlet_grants (secret_id, role)
    WHERE role IS NOT NULL;

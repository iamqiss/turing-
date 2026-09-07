-- Domain discovery support + ephemeral OIDC/SAML login state

CREATE INDEX IF NOT EXISTS org_sso_configs_domains_gin
    ON org_sso_configs USING GIN (domains);

CREATE TABLE IF NOT EXISTS sso_login_states (
    state           TEXT PRIMARY KEY,
    org_id          UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    protocol        TEXT NOT NULL DEFAULT 'oidc',
    code_verifier   TEXT,
    nonce           TEXT,
    redirect_uri    TEXT NOT NULL,
    email_hint      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS sso_login_states_expires_idx ON sso_login_states (expires_at);

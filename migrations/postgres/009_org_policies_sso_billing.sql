-- Repair: ensure workspaces.org_id exists even if 008 partially applied
ALTER TABLE workspaces
    ADD COLUMN IF NOT EXISTS org_id UUID REFERENCES orgs(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS workspaces_org_idx ON workspaces (org_id);

-- Org-level default repository policies (cascade to new repos)
CREATE TABLE IF NOT EXISTS org_settings (
    org_id                      UUID PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    allowed_merge_methods       TEXT[] NOT NULL DEFAULT ARRAY['merge','squash','rebase'],
    default_merge_method        TEXT NOT NULL DEFAULT 'merge'
        CHECK (default_merge_method IN ('merge', 'squash', 'rebase')),
    delete_branch_on_merge      BOOLEAN NOT NULL DEFAULT FALSE,
    require_up_to_date          BOOLEAN NOT NULL DEFAULT FALSE,
    restrict_merge_to_admins    BOOLEAN NOT NULL DEFAULT FALSE,
    required_approvals          INT NOT NULL DEFAULT 0,
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER org_settings_updated_at
    BEFORE UPDATE ON org_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- SSO config (OIDC) per org
CREATE TABLE IF NOT EXISTS org_sso_configs (
    org_id              UUID PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    enabled             BOOLEAN NOT NULL DEFAULT FALSE,
    provider            TEXT NOT NULL DEFAULT 'oidc',
    issuer_url          TEXT,
    client_id           TEXT,
    client_secret       TEXT,
    domains             TEXT[] NOT NULL DEFAULT '{}',
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER org_sso_configs_updated_at
    BEFORE UPDATE ON org_sso_configs
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Stripe linkage on orgs (already have billing_customer_id)
ALTER TABLE orgs
    ADD COLUMN IF NOT EXISTS stripe_subscription_id TEXT;
ALTER TABLE orgs
    ADD COLUMN IF NOT EXISTS billing_status TEXT NOT NULL DEFAULT 'none';

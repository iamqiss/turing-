CREATE TABLE IF NOT EXISTS orgs (
    id                      UUID PRIMARY KEY,
    name                    TEXT NOT NULL,
    slug                    TEXT NOT NULL,
    seat_limit              INT NOT NULL DEFAULT 5,
    billing_customer_id     TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS orgs_slug_idx ON orgs (lower(slug));

CREATE TRIGGER orgs_updated_at
    BEFORE UPDATE ON orgs
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE IF NOT EXISTS org_members (
    org_id          UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'billing')),
    seat_status     TEXT NOT NULL DEFAULT 'active'
        CHECK (seat_status IN ('active', 'invited', 'suspended')),
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, principal_id)
);

CREATE INDEX IF NOT EXISTS org_members_principal_idx ON org_members (principal_id);

CREATE TABLE IF NOT EXISTS org_invitations (
    id              UUID PRIMARY KEY,
    org_id          UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    email           TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'billing')),
    token           TEXT NOT NULL UNIQUE,
    invited_by      UUID NOT NULL REFERENCES principals(id),
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS org_invitations_org_idx ON org_invitations (org_id);
CREATE INDEX IF NOT EXISTS org_invitations_email_idx ON org_invitations (lower(email));

-- Attach workspaces to orgs (nullable for legacy rows until backfilled)
ALTER TABLE workspaces
    ADD COLUMN IF NOT EXISTS org_id UUID REFERENCES orgs(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS workspaces_org_idx ON workspaces (org_id);

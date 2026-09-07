
-- SAML + OIDC SSO config expansion (SSO wall: Business/Enterprise only in product)

ALTER TABLE org_sso_configs
    ADD COLUMN IF NOT EXISTS protocol TEXT NOT NULL DEFAULT 'oidc'
        CHECK (protocol IN ('oidc', 'saml')),
    ADD COLUMN IF NOT EXISTS saml_entity_id TEXT,
    ADD COLUMN IF NOT EXISTS saml_sso_url TEXT,
    ADD COLUMN IF NOT EXISTS saml_slo_url TEXT,
    ADD COLUMN IF NOT EXISTS saml_idp_x509 TEXT,
    ADD COLUMN IF NOT EXISTS saml_sp_entity_id TEXT,
    ADD COLUMN IF NOT EXISTS enforce_sso BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS jit_provisioning BOOLEAN NOT NULL DEFAULT TRUE;

COMMENT ON TABLE org_sso_configs IS 'Org SSO (OIDC/SAML). Enabling requires Business+ plan (SSO wall).';

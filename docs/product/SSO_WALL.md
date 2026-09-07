# SSO Wall — enterprise monetization

## Product rule

| Plan | Passkey / passphrase | OIDC SSO | SAML SSO | Enforce SSO | SCIM |
|------|----------------------|----------|----------|-------------|------|
| Free | yes | no | no | no | no |
| Team | yes | no | no | no | no |
| **Business** | yes | **yes** | **yes** | yes | no |
| **Enterprise** | yes | **yes** | **yes** | yes | **yes** |

Enabling org SSO without Business/Enterprise returns **402 Payment Required** with `code: sso_wall`.

## Why it converts

- Mid-market and enterprise IT **require** IdP login (Okta, Azure AD, Google Workspace).
- Seat price alone under-monetizes security/compliance buyers.
- SSO is low COGS relative to perceived value → high-margin upgrade.

## Technical surface

- Config: `org_sso_configs` (OIDC + SAML fields in `032_sso_saml.sql`)
- Gate: `BillingService::assert_sso_entitled` / `PlanId::includes_sso`
- Admin UI: show upgrade CTA when `sso_allowed == false`
- API: `GET /api/orgs/:slug/billing/sso-entitlement`

## Login (roadmap)

1. Domain discovery: email domain → org SSO config  
2. OIDC: authorization code + PKCE  
3. SAML: SP-initiated ACS  
4. JIT user provisioning when `jit_provisioning`  
5. Optional `enforce_sso` blocks passphrase for domain members (Business+)

## Pricing narrative

Keep Team attractive for startups; put SSO behind Business so “we need Okta” becomes an expansion trigger, not a free feature.

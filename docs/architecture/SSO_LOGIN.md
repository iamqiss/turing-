# SSO login — domain discovery & OIDC

## Domain discovery

```
POST /api/auth/sso/discover
{ "email_or_domain": "alice@acme.com" }
# or "acme.com"
```

Looks up `org_sso_configs.domains` (case-insensitive). If the org plan is under the **SSO wall**, returns `found: false` with an upgrade message.

## OIDC (PKCE)

1. `POST /api/auth/sso/oidc/start` `{ "email_or_domain": "..." }`  
   → `{ authorization_url, state }`  
2. Browser follows IdP  
3. IdP redirects to `GET /api/auth/sso/oidc/callback?code&state`  
4. TT exchanges code, reads `id_token`, JIT-provisions if enabled, creates PASETO session  
5. Redirect `/auth/sso/complete#token=...&handle=...`

## Configure org (admin, Business+)

```
PUT /api/orgs/{slug}/sso
{
  "enabled": true,
  "issuer_url": "https://login.microsoftonline.com/.../v2.0",
  "client_id": "...",
  "client_secret": "...",
  "domains": ["acme.com"]
}
```

Register the callback URL on the IdP:
`https://<tt-host>/api/auth/sso/oidc/callback`

## Next

- SAML ACS (`protocol=saml`)
- Signature verification of ID tokens (JWKS)
- Enforce SSO for domain members on login UI

## JWKS (OIDC ID token)

After code exchange, TT:

1. Reads `kid` from JWT header
2. Fetches IdP `jwks_uri` from discovery document
3. Matches JWK (`RSA` n/e or `EC` x/y)
4. Verifies signature with `jsonwebtoken`
5. Validates `iss`, `aud` (client_id), `exp`, and **nonce** (must match start)

Unsigned / wrong-issuer tokens are rejected (`401`).

## SAML ACS

| Step | Endpoint |
|------|----------|
| Start | `POST /api/auth/sso/saml/start` `{ "email_or_domain" }` → `{ redirect_url, state }` |
| ACS | `POST /api/auth/sso/saml/acs` form: `SAMLResponse`, `RelayState` |

- Builds AuthnRequest (HTTP-Redirect, deflate+base64)
- ACS decodes assertion, extracts NameID / email attributes
- JIT + session same as OIDC
- Register ACS URL on IdP: `https://<host>/api/auth/sso/saml/acs`

Set `protocol: saml` and `saml_sso_url` / `saml_sp_entity_id` / `saml_idp_x509` on org SSO config.
Full XML-DSig verification against `saml_idp_x509` is the next hardening step when cert is present.


## JWKS cache

OIDC JWKS documents are cached in-process for **1 hour**. On unknown `kid`, cache is invalidated and refetched once.

# Scarlet Letter ✉

> Kills `.env`. Org-scoped, IDE-aware, Pearl-aware secrets.  
> Spelling: **Scarlet** (not Scarlett).

## Why

Anything in a model context window is a potential leak. Pearl must know `DATABASE_URL` *exists* and its shape — never the value.

## Scopes

| Scope | Meaning |
|-------|---------|
| **Org** | Shared across the organization |
| **Environment** | `dev` / `staging` / `prod` |
| **Repository** | Single repo |

Resolution for pull with `environment=prod`: env-specific first, then org-level fallback.

## Envelope encryption

1. Random **DEK** per secret write  
2. DEK encrypts plaintext (AES-256-GCM)  
3. DEK wrapped by **KEK** via `KeyProvider`  
4. Store only: packed `wrapped_dek` + `nonce` + `ciphertext`

Plaintext exists only ephemerally in authorized pull paths (CLI / CI inject).

## KMS — in-house (`tt-kms`)

Scarlet Letter does **not** depend on AWS KMS or Vault for production. KEK lives in **`tt-kms`** on Turing Tarpit bare metal (Latitude / OVH / …), private network only.

```
┌─────────────┐  wrap/unwrap DEK   ┌──────────────┐
│  tt-server  │ ─────────────────► │   tt-kms     │  KEK in RAM after unseal
│  (Scarlet)  │   HMAC + private   │  (TT metal)  │  sealed blob on local disk
└─────────────┘       NIC          └──────────────┘
```

| `TT_KMS_PROVIDER` | When |
|-------------------|------|
| **`tt`** | Production — talk to `tt-kms` |
| **`local`** | Lab / Termux single-process only |
| **`http`** | Optional generic HTTP (not the product path) |

```bash
# KMS node
export TT_KMS_BIND=10.0.0.5:9797
export TT_KMS_UNSEAL='…'
export TT_KMS_AUTH_SECRET='…'
export TT_KMS_SEALED_PATH=/var/lib/tt-kms/master.sealed
export TT_KMS_KEY_ID=scarlet-v1
cargo run -p tt-kms

# Control plane
export TT_KMS_PROVIDER=tt
export TT_KMS_ENDPOINT=http://10.0.0.5:9797
export TT_KMS_KEY_ID=scarlet-v1
export TT_KMS_TOKEN=$TT_KMS_AUTH_SECRET
```

API: `GET /v1/health` · `POST /v1/wrap` · `POST /v1/unwrap`

## API (control plane)

```http
POST   /api/orgs/:slug/secrets
GET    /api/orgs/:slug/secrets
GET    /api/orgs/:slug/secrets/:name    # metadata / Pearl probe
POST   /api/orgs/:slug/secrets/pull     # values — CLI/CI only
DELETE /api/orgs/:slug/secrets/:name
POST   /api/secrets/scan                # pre-commit helper
```

## Pearl integration

- Tools: `scarlet_probe`, `scarlet_list_secrets` (metadata), gated put  
- **On-device redaction** before every inference call (`redact_for_model`)  
- Placeholders: `[REDACTED:<rule> len=N]` — model must not invent values  

## Push protection

- Pattern scanner: AWS keys, GitHub PATs, Slack, Stripe, JWTs, connection strings, high-entropy assignments, TT deploy tokens  
- Pre-receive hook path + client `/api/secrets/scan`  
- Repo settings: `block_secret_push`, `secret_push_allowlist`  

## Local CLI contract (target)

```bash
tt secrets pull                  # materialize map (not a file)
tt secrets run -- npm start      # env only for child process
```

IDE / Pearl: local authenticated socket for “exists + type”, never value in context.

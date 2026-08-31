# Scarlet Letter ✉

> Kills `.env`. Org-scoped, IDE-aware, Pearl-aware secrets.
> Spelling: **Scarlet** (not Scarlett).

## Why

Anything that lands in a model context window is a potential leak.
Pearl (TT’s agent) must know `DATABASE_URL` **exists** and is a `connection`-shaped secret —
without ever seeing the value.

## Scopes (GitHub Actions mental model, usable locally)

| Scope | When |
|-------|------|
| **Org** | Shared across the org |
| **Environment** | `dev` / `staging` / `prod` (or custom) |
| **Repository** | Single repo |

Resolution for pull with `environment=prod`: env-specific first, then org-level fallback.

## Envelope encryption

1. Random **DEK** per secret write
2. DEK encrypts plaintext (AES-256-GCM)
3. DEK wrapped by server **KEK** (`TT_SECRETS_KEY` — KMS-backed later)
4. Store: `wrapped_dek` + `nonce` + `ciphertext` only

Turing Tarpit never persists plaintext. Decrypt only in authorized pull paths (CLI / CI).

## API

```http
POST /api/orgs/:slug/secrets          # put/rotate (admin)
GET  /api/orgs/:slug/secrets          # list meta (members)
GET  /api/orgs/:slug/secrets/:name    # Pearl probe (exists + kind)
POST /api/orgs/:slug/secrets/pull     # map of values for CLI/CI only
DELETE /api/orgs/:slug/secrets/:name  # admin
```

Query: `?environment=prod&repository_id=...`

Pull body:
```json
{ "names": ["DATABASE_URL", "REDIS_URL"], "environment": "dev", "client_kind": "cli" }
```

## Local CLI (planned / contract)

```bash
tt secrets pull --org acme --env dev          # into process memory cache
tt secrets run --env dev -- npm start         # wrap process; no file written
tt secrets rotate DATABASE_URL --env prod
```

IDE / Pearl: local authenticated socket asks **meta only** (`exists`, `value_kind`).

## Pearl tools

| Tool | Sees values? |
|------|----------------|
| `scarlet_probe` | No |
| `scarlet_list_secrets` | No |
| `scarlet_put_secret` | Write path only; response is meta; **requires approval** |

## Guardrails (roadmap hooks)

- Pre-commit + push protection: block secret-shaped strings
- Pearl policy: redact known secret **refs** and high-entropy values from prompt/completion before leave-machine
- Audit: `scarlet_access_audit` — who pulled / rotated / probed, when, client_kind

## Migration

`018_scarlet_letter.sql` — replaces the interim workspace-only `scarlett_secrets` table concept.


## Push protection & pre-commit

**Server (default on):** every bare repo gets a `pre-receive` hook that scans new commits for secret-shaped strings (private keys, AWS keys, GitHub PATs, Stripe keys, connection URLs, `tt_deploy_` tokens, password assignments). Matching pushes are **rejected**.

**Repo settings:** `block_secret_push` (default true), `secret_push_allowlist` (rule ids to ignore).

**Local pre-commit:**
```bash
curl -s http://localhost:8080/api/secrets/pre-commit-hook > .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

**Scan API (IDE / CI):**
```http
POST /api/secrets/scan
{ "content": "...", "allowlist": ["high_entropy_assign"] }
```


## On-device redaction (Pearl)

Before every inference call, `redact_for_model` runs on the full message list:

1. Secret-shaped patterns → `[REDACTED:<rule> len=N]`
2. `ENV_NAME=longvalue` lines → `ENV_NAME=[REDACTED:env]`
3. Tool args fields `value` / `password` / `secret` / `token` → opaque placeholders
4. Tool results re-fed into context are redacted again

The model never sees live secret material. Pearl may only learn *that* a secret exists via `scarlet_probe`.


## KMS provider (in-house)

Scarlet Letter does **not** depend on AWS KMS, Vault, or other vendors for production.

KEK lives in **`tt-kms`**, run on Turing Tarpit bare metal (Latitude.sh, OVHcloud, …), private network only.

```
┌─────────────┐  wrap/unwrap DEK   ┌──────────────┐
│  tt-server  │ ─────────────────► │   tt-kms     │  KEK in RAM after unseal
│  (Scarlet)  │   HMAC + private   │  (TT metal)  │  sealed blob on local disk
└─────────────┘       NIC          └──────────────┘
```

| Provider | When |
|----------|------|
| **`tt`** | Production — `tt-kms` binary on TT hardware |
| **`local`** | Lab / Termux single-process only |
| **`http`** | Optional generic HTTP (not preferred) |

```bash
# On KMS node (private IP)
export TT_KMS_BIND=10.0.0.5:9797
export TT_KMS_UNSEAL='operator-passphrase'
export TT_KMS_AUTH_SECRET='shared-with-control-plane'
export TT_KMS_SEALED_PATH=/var/lib/tt-kms/master.sealed
cargo run -p tt-kms

# Control plane
export TT_KMS_PROVIDER=tt
export TT_KMS_ENDPOINT=http://10.0.0.5:9797
export TT_KMS_KEY_ID=scarlet-v1
export TT_KMS_TOKEN=$TT_KMS_AUTH_SECRET
```

KEK never leaves `tt-kms`. Control plane only handles DEK wrap blobs + data ciphertext.

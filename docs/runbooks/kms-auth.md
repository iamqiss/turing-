# tt-kms request authentication (v2)

`/v1/wrap` and `/v1/unwrap` hand out DEKs, so every call is HMAC-signed.
`/v1/health` stays unauthenticated for probes.

## Wire format

Headers on every wrap/unwrap request:

| Header | Value |
|--------|-------|
| `x-tt-kms-timestamp` | unix seconds |
| `x-tt-kms-nonce` | 16–64 alphanumeric chars, unique per request (client sends 32 hex) |
| `x-tt-kms-signature` | hex `HMAC-SHA256(secret, signing_input)` |

```
signing_input = "tt-kms-v2\n{ts}\n{nonce}\n{METHOD}\n{path}\n{hex(sha256(body))}"
```

The server rejects a request when the timestamp is more than 60 s from its own
clock, the signature does not match (constant-time compare), or the nonce was
already used inside the window. Run NTP on both ends.

## Operator changes (breaking)

- Deploy `tt-kms` and the control plane **together**. v1 clients (timestamp-only
  signature) are rejected.
- `TT_KMS_AUTH_SECRET` is now **required** and must be at least 32 bytes
  (`openssl rand -hex 32`). `tt-kms` refuses to start without it.
- Local lab only: `TT_KMS_INSECURE_LAB=1` allows an empty secret and serves
  wrap/unwrap unauthenticated. Never set it on metal.
- The replay cache is in memory. Restarting `tt-kms` clears it, which is safe
  because the 60 s timestamp window still applies.

## Code

- Server: `crates/tt-kms/src/api.rs` (`signing_input`, `verify_request`)
- Client: `crates/tt-secrets/src/kms.rs` (`signing_input`, `sign_request`)
- Both crates assert the same golden vector; if you change the format, change
  both and the vector.

## Sealed master key (`TT_KMS_SEALED_PATH`)

New seals use **Argon2id** (64 MiB, 3 passes, 1 lane) with the KDF header
authenticated as AES-GCM associated data. Files from older versions (a single
SHA-256 of the passphrase) still open and are upgraded on boot:

1. the old file is copied to `master.sealed.v1.bak`
2. the new v2 blob is verified to open to the same key
3. `master.sealed` is atomically replaced (mode 0600)

Restart once to confirm, then **delete `master.sealed.v1.bak`**. It keeps the
weak format, so anyone who copies it can brute-force your passphrase. Choose a
long random `TT_KMS_UNSEAL` passphrase regardless.

Downgrading to a tt-kms build that predates v2 will not read the new file; restore
the `.v1.bak` first if you need to roll back.

## Local provider and the dev key

`TT_KMS_PROVIDER=local` (the default) now **requires `TT_SECRETS_KEY`**
(`openssl rand -hex 32`); `tt-server` refuses to boot without it, and a malformed
key is an error instead of silently becoming an all-zero key.

For a throwaway lab you may set `TT_ALLOW_INSECURE_DEV_KEY=1` to use the
publicly known key from the source tree. Data written that way is readable by
anyone who has the database, so never use it with real secrets. If you have lab
data encrypted with the old implicit dev key, set that flag once to keep reading
it, then re-encrypt under a real key.

Note: the compute service also accepts `TT_SESSION_KEY` as its secrets key when
`TT_SECRETS_KEY` is empty, but the local Scarlet provider reads `TT_SECRETS_KEY`
only. Set `TT_SECRETS_KEY` explicitly.

## Rotating the secrets KEK

Each secret records the provider and key id it was sealed under. New and
updated secrets always use the **active** key; reading a secret uses the key it
was sealed under, so rotation no longer breaks old data **as long as the old key
is still available**.

Local provider:

```bash
# before: TT_SECRETS_KEY=<old>  TT_KMS_KEY_ID=scarlet-v1
export TT_SECRETS_KEY="$(openssl rand -hex 32)"
export TT_KMS_KEY_ID=scarlet-v2
export TT_SECRETS_RETIRED_KEYS="scarlet-v1:<old 64-hex key>"   # comma-separate several
```

Moving from `local` to `tt`: set `TT_KMS_PROVIDER=tt` and keep the old local key
in `TT_SECRETS_RETIRED_KEYS` (use the id it had, normally `scarlet-v1`).

Rules:

- Do not delete a retired key until no secret still depends on it. Updating a
  secret (`put`) re-seals it under the active key. There is no bulk re-seal job
  yet; `Envelope::needs_rewrap` is the building block for one.
- If a secret cannot be opened the error names the provider/key it was sealed
  under and what is configured, e.g. `cannot unwrap DEK sealed under local/scarlet-v1;
  configured: local/scarlet-v2 (active)`. That means a retired key is missing.
- The key id is a label; the key material decides. A renamed label with the same
  key still opens (a warning is logged).
- `tt` provider: the control plane now asks tt-kms to unwrap with the recorded
  key id, but `tt-kms` currently loads a single master key at boot, so it cannot
  yet serve an old key next to a new one. Rotating tt-kms keys needs that
  server-side support first.

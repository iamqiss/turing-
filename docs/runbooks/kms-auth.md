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

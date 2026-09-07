# Break Glass recovery (hardened)

User-enabled recovery when passphrase access is lost. Methods can be combined.

## Hardening summary

| Control | Behavior |
|---------|----------|
| Ed25519 trustees | `trustee_pubkey` must be 32-byte Ed25519; approvals verify `signature` over `tt-break-glass:v1:{event_id}:{principal_id}` |
| Rate limit | 16 triggers / hour / (handle+IP fingerprint) in `break_glass_rate_limits` |
| Anti-enumeration | Unauthenticated triggers return a **generic** message whether or not the account exists / method is enabled |
| Constant-time tokens | Cancel / recovery tokens compared via SHA-256 + `subtle::ConstantTimeEq` |
| Single-use recovery | `recovery_claimed_at` set on claim; second claim fails |
| Time-lock worker | Every 60s, `tick_time_locks` promotes pending events with `unlock_at <= now` to `ready` |
| Session revoke | Successful complete revokes **all** sessions for the principal |

## Time-lock flow

1. Enable settings + optional notify email/phone.
2. `POST /api/auth/break-glass/time-lock` `{ "handle" }` (optional `X-Forwarded-For`).
3. If started: notifications enqueued; `cancel_token` only when `started: true`.
4. Cancel: account session **or** cancel token.
5. After delay: auto `ready` via tick **or** claim promotes; `POST .../events/:id/claim` once.
6. `POST .../complete` with recovery token + new passphrase.

## Trustees flow

1. Add 3–5 trustees with Ed25519 pubkeys (+ optional linked principal).
2. Trigger recover (generic response if not configured).
3. Each trustee signs the canonical message; linked trustees must also present their session.
4. Threshold → `ready` → claim → complete.

## Production

- Terminate TLS; pass real client IP in `X-Forwarded-For` only from trusted proxies.
- Drain `break_glass_notifications` for email/SMS/push.
- Edge rate limits in addition to app limits.

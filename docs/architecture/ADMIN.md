# Administration

## Layers

| Layer | Who | Scope |
|-------|-----|--------|
| **Org admin** | Org `owner` / `admin` | Members, invites, seats, SSO, policies, workspaces, billing link |
| **Platform operator** | `PLATFORM_OPERATOR_IDS` | Node registration, compute ops |
| **Superadmin** | `SUPERADMIN_IDS` (falls back to operators) | All orgs, suspend, plan overrides, **finance dashboard** |

Set env:

```bash
export SUPERADMIN_IDS="<your-principal-uuid>"
# or
export PLATFORM_OPERATOR_IDS="<your-principal-uuid>"
```

## Superadmin API

```http
GET  /api/platform/admin/me
GET  /api/platform/admin/orgs
GET  /api/platform/admin/orgs/:slug
POST /api/platform/admin/orgs/:slug/suspend     { "reason": "..." }
POST /api/platform/admin/orgs/:slug/unsuspend
POST /api/platform/admin/orgs/:slug/plan        { "plan": "team|business|...", "seat_quantity": N }
GET  /api/platform/admin/finance?period=2026-09
GET  /api/platform/admin/finance/ledger?period=2026-09
```

Finance snapshot: seat + usage revenue micros, active orgs/seats. Ledger reads `billing_ledger_entries`.

## Org admin (existing)

```http
GET/POST /api/orgs ...
GET  /api/orgs/:slug/billing          # seat snapshot
GET  /api/orgs/:slug/billing/account  # BillingService account
GET  /api/orgs/:slug/billing/invoice-preview
POST /api/orgs/:slug/members ...
POST /api/orgs/:slug/seat-limit
PUT  /api/orgs/:slug/sso
```

## Cloud admin (next steps)

- Managed resource provision (TuringCache / Store / Chronos) under org admin
- Audit log of superadmin actions
- Hard suspend: block compute create + git push when status suspended


## Hard enforcement

When `billing_accounts.status` is `suspended`, `canceled`, `unpaid`, or `past_due`:

| Action | Allowed? |
|--------|----------|
| Git clone / fetch | yes |
| Git **push** | **no** (403) |
| Compute **create** / **start** | **no** (403) |
| Compute stop / destroy | yes (cleanup) |

`BillingService::assert_org_active` · `org_allows_push` (Smart HTTP).

Superadmin actions write `admin_audit_log` (`GET /api/platform/admin/audit`).

See also [FINANCE.md](./FINANCE.md) for MRR/ARR, expenses, tax.

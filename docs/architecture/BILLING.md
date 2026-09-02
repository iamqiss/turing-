# Billing — seats + usage (do not conflate)

> Three chatty humans can burn more **compute** than fifty who only push code.
> Seats fund collaboration. Meters fund variable cost. Mixing them bankrupts TT.

## Dual track

| Track | Pays for | Priced how |
|-------|----------|------------|
| **Seats** | Messaging, Rooms, git hosting UX, issues/PRs, search, Scarlet meta | Per active human / month |
| **Usage** | Compute vCPU/memory, git storage/transfer, media, managed data | Meters + plan-included allowance + overage |

Agents do **not** consume seats (automation identity). Their compute still meters.

## Plans (list USD)

| Plan | Seat / mo | Notes |
|------|-----------|--------|
| Free | $0 (≤3 seats) | Tiny included compute/storage |
| Team | **$10** | Undercuts GH Team (~$4) + Slack Pro (~$8.75) *sum* |
| Business | **$18** | SSO surface, higher includes |
| Enterprise | **$30** list | Negotiated includes / support |

Included vCPU-hours etc. are **allowances**, not unlimited compute.

## Meters

| Meter | Unit | List overage (indicative) |
|-------|------|---------------------------|
| `compute.vcpu_hours` | vCPU-hour | ~$0.04 |
| `compute.memory_gb_hours` | GB-hour | ~$0.005 |
| `git.storage_gb_month` | GB-month | ~$0.015 |
| `git.transfer_gb` | GB | ~$0.08 |
| `msg.storage_gb_month` | GB-month | ~$0.02 |
| `media.storage_gb_month` | GB-month | ~$0.015 |
| `managed.postgres_hours` | instance-hour | ~$0.05+ by SKU |
| `managed.ember_hours` | instance-hour | EmberDB (TT brand) |
| `managed.rustfs_gb_month` | GB-month | Object capacity |
| `managed.greptime_hours` | instance-hour | Time-series |

Amounts in code are **USD micros** (`Catalog::default_list`). Override via `billing_price_overrides` when needed.

## CloudRat / metal margin

- Bare metal ~R1k–R2k/mo class hosts; **4:1 overcommit** on sellable vCPU.
- List compute price must sit **above** amortized power+network+support after overcommit, **below** big-cloud on-demand.
- Seat revenue should cover control-plane + support; usage revenue must not go negative on heavy tenants.

## Managed data

See [MANAGED_DATA.md](./MANAGED_DATA.md).

| Product | Branding | Meter |
|---------|----------|--------|
| PostgreSQL | Unbranded | `managed.postgres_hours` |
| **TuringCache** | Powered by Ember (Kacy Fortner @kacyf) | `managed.cache_hours` |
| **TuringStore** | Powered by RustFS | `managed.store_gb_month` |
| **TuringChronos** | Powered by GreptimeDB | `managed.chronos_hours` |

UI always shows powered-by credits (no separate vanilla SKUs).


## API

```http
GET  /api/billing/catalog
GET  /api/orgs/:slug/billing
GET  /api/orgs/:slug/billing/invoice-preview?period=2026-09
POST /api/orgs/:slug/billing/usage          # admin/lab; prod: internal emitters
POST /api/billing/stripe/webhook
GET  /api/platform/billing/finance?period=2026-09   # platform operators
```

## Schema

Migration `023_billing.sql`: accounts, usage events, monthly rollups, managed resources, ledger.

## Hard rules

1. **Never** raise seat price to "cover" unlimited compute.
2. Heavy compute tenants pay usage — even on Free/Team after allowance.
3. Soft alerts before hard stop; hard stop on severe past_due for compute provision.
4. Platform finance view = seats + usage revenue (cost COGS tracked in ledger later).

## Crate

`tt-billing` — catalog, meters, `BillingService`, Postgres store.

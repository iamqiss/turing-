# Finance — revenue, cash, expenses, invoices

## Two revenue views (do not confuse them)

| View | Source | Use |
|------|--------|-----|
| **Catalog / metered (accrual)** | Seats × list + usage overage | MRR / ARR estimates, product pricing |
| **Stripe cash** | Webhooks `invoice.paid`, `charge.succeeded` | Real money collected |

If seats look fine but **expenses ↑** and **cash** stays flat, fix **usage pricing** or COGS — do not paper over with seat discounts.

## Usage emitters

| Event | Meter |
|-------|--------|
| Workload stop | `compute.vcpu_hours` |
| Media attachment | `media.storage_gb_month` |
| `POST .../jobs/git-storage-scan` | `git.storage_gb_month` per org |
| Managed tick | cache / chronos / store meters |

## Invoices

```http
GET  /api/orgs/:slug/billing/invoice-preview?period=2026-09
POST /api/orgs/:slug/billing/invoices/issue   { "period": "2026-09" }
POST /api/orgs/:slug/billing/invoices/pay     { "period": "2026-09", "cash_micros": 0 }
```

Issue rolls seats + usage + tax (from `billing_tax_settings`) into `billing_invoices` status `open`.  
Pay marks `paid` and records cash. Stripe `invoice.paid` also marks paid and appends `billing_stripe_events`.

## Stripe reconciliation

Webhook `/api/billing/stripe/webhook` handles:

- `customer.subscription.*` → seats / plan status  
- `invoice.paid` / `charge.succeeded` → cash event + mark invoice paid  
- `invoice.payment_failed` → org `past_due` (hard enforcement applies)

Idempotent on `stripe_event_id`.

## Superadmin finance

```http
GET  /api/platform/admin/finance?period=2026-09
POST /api/platform/admin/expenses
GET  /api/platform/admin/expenses?period=2026-09
POST /api/platform/admin/jobs/git-storage-scan
```

Finance JSON includes `revenue`, `mrr_micros`, `arr_micros`, `expenses`, `gross_margin_*`, `tax`, `invoices`, **`stripe_cash_micros`**.

## Book COGS

Categories: `metal` · `power` · `network` · `storage` · `software` · `support` · `tax` · `other`  
Amounts in **USD micros** (1_000_000 = $1).

## Schema

- `026_finance_cogs.sql` — expenses, tax, invoices  
- `027_stripe_invoices_git.sql` — stripe events, cash columns, git snapshots  

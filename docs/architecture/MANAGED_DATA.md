# Managed data products

Turing Tarpit offers **Turing-branded** managed data only. The UI always shows **what powers each product** so customers know TT runs the same underlying technology.

## Catalog

| Product | Powered by | Role |
|---------|------------|------|
| **PostgreSQL** | PostgreSQL (unbranded) | Primary OLTP / app DB |
| **TuringCache** | [Ember](https://github.com/kacy/ember) by **Kacy Fortner ([@kacyf](https://x.com/kacyf))** | Redis-compatible distributed cache |
| **TuringStore** | [RustFS](https://rustfs.com/) | S3-compatible object storage |
| **TuringChronos** | [GreptimeDB](https://github.com/GreptimeTeam/greptimedb) | Metrics, logs, traces |

No separate “vanilla” SKU line — one offer, transparent stack.

## UI copy pattern

```
TuringCache
Powered by Ember by Kacy Fortner (@kacyf)
```

```
TuringStore
Powered by RustFS — with thanks to the RustFS team
```

```
TuringChronos
Powered by GreptimeDB — with thanks to the Greptime team
```

Code: `ManagedKind::display_name()`, `powered_by()`, `attribution()`.

## Why these engines

- **Ember** — RESP3 Redis drop-in, shard-per-core, Raft/SWIM, encryption at rest; sessions, queues, agent scratch.
- **RustFS** — Apache-2.0 S3 API, Rust core; media, LFS, artifacts, Chronos object backend.
- **GreptimeDB** — metrics + logs + traces, PromQL + SQL, object-storage primary; platform and tenant telemetry.

## Billing meters

| Kind | Meter | Unit |
|------|--------|------|
| Postgres | `managed.postgres_hours` | instance-hour |
| TuringCache | `managed.cache_hours` | instance-hour |
| TuringStore | `managed.store_gb_month` | GB-month |
| TuringChronos | `managed.chronos_hours` | instance-hour |

## Schema

`billing_managed_resources.kind` — migration `024_managed_branding.sql`.

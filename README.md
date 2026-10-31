# Turing Tarpit

**Last updated: 2026-09-03** — Proprietary self-hosted developer OS: Git + messaging + orgs + billing + compute + secrets + agents, controlled by a Rust modular monolith.

Client UIs live **outside** this repository (Compose Multiplatform app: separate `TuringTarpit` project). This tree is the **backend control plane** only.

Docs: [Architecture](docs/architecture/TURING_TARPIT.md) · [Scarlet Letter](docs/architecture/SCARLET_LETTER.md) · [Media](docs/architecture/MEDIA.md) · [Termux lab](docs/runbooks/termux-lab.md) · [API / client contract](docs/api/) (see also CMP `docs/CLIENT_CONTRACT.md`)

---

## What it is

A clean-room, centralized workspace for humans and AI agents — Git hosting, channels/DMs/Rooms, issues/PRs, orgs & seats, metered usage, Firecracker-oriented compute, and envelope-encrypted secrets — without federated protocol tax.

- Humans and agents share channels, permissions, and an audit trail.
- Conversation and code in one place (Smart HTTP Git via Gitoxide).
- Self-hosted first; modular monolith so realtime can move Redis → **Apache Iggy** (or Kafka) without rewriting product code.
- Agents (Pearl) are first-class; Scarlet Letter keeps secrets out of model context and off disk in plaintext.

## Stack (backend)

| Layer | Choice |
|-------|--------|
| Language | Rust |
| HTTP API | Axum (REST + Git Smart HTTP) |
| RPC | tonic gRPC + **tonic-web** (Auth, Messaging today) |
| Primary DB | PostgreSQL (SQLx) |
| Cache / sessions / ceremony | Redis |
| Analytics / activity blend | ClickHouse |
| Search | Meilisearch |
| Git | Gitoxide + Smart HTTP |
| Object storage | S3-compatible (TuringStore / RustFS lineage) |
| Auth | WebAuthn (`webauthn-rs`) + PASETO sessions + agent API tokens |
| Secrets | Scarlet Letter — envelope encryption, in-house KMS (`tt-kms`) |
| Compute | Node registration, quotas, lifecycle events (Firecracker / Cloud Hypervisor oriented) |
| Pipelines | Turing Tasks (`.tt/pipelines/*.yml`), billed as `tasks.minutes` |
| Realtime | In-memory + optional **Iggy** (`TT_REALTIME_BACKEND=iggy`) |

## Brand (product)

| Token | Value | Role |
|-------|--------|------|
| Orange | `#ff3c00` | Primary accent / CTAs |
| Pitch black | `#0a0a0a` | Chrome / density |
| Surfaces | light neutrals | Content areas |

(Legacy ivory / mint / grey remain as secondary tokens in older docs.)

## Workspace layout

```
turing-tarpit/
├── crates/
│   ├── tt-server         # Binary: HTTP + gRPC listeners, route composition
│   ├── tt-api            # Protobuf generated types / service traits
│   ├── tt-core           # Domain types, IDs, errors
│   ├── tt-common         # Shared utilities
│   ├── tt-config         # Config loading
│   ├── tt-auth           # WebAuthn, PASETO sessions, agent tokens
│   ├── tt-messaging      # Channels, messages, threads, DMs, Rooms
│   ├── tt-git            # Repos, Smart HTTP, PRs, protections
│   ├── tt-search         # Meilisearch indexing / query
│   ├── tt-realtime       # Fanout; Iggy dual-write optional
│   ├── tt-billing        # Seats, meters, catalog, Stripe hooks
│   ├── tt-compute        # Nodes, workloads, quotas, lifecycle
│   ├── tt-node           # Node agent side
│   ├── tt-net            # Addressing / NAT helpers
│   ├── tt-secrets        # Scarlet Letter
│   ├── tt-kms            # In-house KEK/DEK
│   ├── tt-storage        # Object store trait + impls
│   ├── tt-pipelines      # Turing Tasks
│   ├── tt-agents         # Pearl / agent runtime adapters
│   ├── tt-notifications
│   ├── tt-audit
│   └── tt-kms …
├── proto/                # auth, messaging, git, search, agents, admin, …
├── migrations/           # Postgres (+ ClickHouse where applicable)
├── config/
├── deploy/
├── scripts/
└── docs/
```

Legacy `clients/` Dioxus experiments (if present) are **not** the supported UI path; do not treat them as product surface.

## What is built (backend)

### Identity & access
- WebAuthn registration/authentication ceremonies (Redis ceremony store)
- PASETO-backed sessions; agent API tokens with scopes
- Dev bootstrap (`POST /api/dev/bootstrap` when `TT_DEV_BOOTSTRAP=1`)
- Org membership, roles, cascade of default repo policies
- Platform operator allowlist hooks for sensitive compute APIs
- Hardening notes: mTLS / signed node challenges, encrypted deploy tokens (roadmap items tracked in compute docs)

### Git
- Create/list repos, Smart HTTP push/pull
- Pull requests, reviews, merge methods, branch protections
- Issues, labels, comments; `Fixes #N` on merge
- Repo settings / enforceable single merge method

### Messaging & community
- Channels, messages, reactions, threads
- 1:1 DMs
- **Rooms** — public/discoverable community spaces (feed, attachments; live video deferred)
- Realtime fanout (memory; Iggy pluggable)

### Profiles & search
- Richer activity blend oriented profiles (transparency over vanity heatmaps)
- Meilisearch: people on profile upsert, messages on post, reindex job, tenant filters

### Orgs & billing
- Orgs, seats, SSO hooks
- Seat + usage meters (git storage, compute, **tasks.minutes**, messaging-related usage, managed data services)
- Stripe webhooks, catalog, invoices / reconciliation paths
- Superadmin finance visibility (MRR/ARR-style reporting surface)

### Scarlet Letter (secrets)
- Org / repo / environment scoped secrets
- Envelope encryption via in-house KMS (no third-party KMS dependency)
- Pull into process env (not plaintext files); audit of pulls
- Pre-commit / push protection for secret-shaped strings
- Policy: redact secret values before Pearl context

### Compute & pipelines
- Node registration, shared NAT, deploy, instance lifecycle events  
  (`node_ready`, `nat_allocated`, `quota_exceeded`, `instance_failed`, …)
- Org compute quotas
- **Turing Tasks** — in-repo `.tt/pipelines/*.yml`, runs metered by minutes

### Media
- Profile and channel media via object store
- Thumbnails + virus scan hooks

### Agents
- Pearl-oriented adapters; tool execution paths constrained by permissions
- Strict human approval gates for dangerous agentic actions (policy direction)

## Transports

| Listener | Port (default) | Surface |
|----------|----------------|---------|
| HTTP (Axum) | `8080` | REST, Git Smart HTTP, health, gRPC-Web-capable paths as configured |
| gRPC (tonic + tonic-web) | `50051` | **AuthService**, **MessagingService** today |

Clients: native **gRPC** on mobile/desktop; **gRPC-Web** on web; REST for workspaces/repos/issues until those services are exposed on gRPC.

## Quick start (lab)

```bash
# Dependencies: Postgres, Redis; optional Meilisearch, ClickHouse, Iggy
export DATABASE_URL=postgres://turing:turing@127.0.0.1/turing_tarpit
export REDIS_URL=redis://127.0.0.1/
export TT_DEV_BOOTSTRAP=1
export TT_SECRETS_KEY="$(openssl rand -hex 32)"   # required; boot fails without it
# throwaway lab only, instead of the line above: export TT_ALLOW_INSECURE_DEV_KEY=1
# optional: TT_REALTIME_BACKEND=iggy

cargo run -p tt-server
```

Bootstrap a session:

```bash
curl -s -X POST http://127.0.0.1:8080/api/dev/bootstrap \
  -H 'content-type: application/json' \
  -d '{"handle":"alice","display_name":"Alice"}'
```

Use `Authorization: Bearer <session_token>` on REST.

## Configuration (common)

| Variable | Purpose |
|----------|---------|
| `DATABASE_URL` | Postgres |
| `REDIS_URL` | Redis |
| `TT_DEV_BOOTSTRAP` | Enable dev bootstrap route |
| `TT_REALTIME_BACKEND` | `memory` (default) or `iggy` |
| `TT_IGGY_*` | Iggy endpoint when enabled |

See `tt-config` and `docs/runbooks/` for full matrices.

## Roadmap (backend-relevant)

- Expand gRPC surface (git, issues, search) beyond auth/messaging
- Iggy as default fanout under load
- Rooms media richness; **Rooms** live A/V still deferred
- Hardened node ↔ control-plane mutual auth
- Managed data products (TuringCache / TuringChronos / TuringStore) provisioning APIs
- Deeper Pearl tool policy + Scarlet integration

**Clients:** Compose Multiplatform (separate project) — not maintained inside this Cargo workspace.

## License

Proprietary. All rights reserved.

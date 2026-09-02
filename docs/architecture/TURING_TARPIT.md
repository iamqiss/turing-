# Turing Tarpit — Architecture & Progress

> **Ironic name, serious product.** A self-hostable, AI-native Developer OS: code hosting, collaboration, compute, secrets, media, and lifecycle transparency — modular Rust monolith, proprietary, backend-first.

**Last updated:** 2026-09-01  
**Status:** Control plane solid through media + Scarlet Letter + in-house KMS; Dioxus clients still ahead.

---

## 1. Vision

Turing Tarpit covers the **full developer lifecycle** so humans *and* peer agents share the same visibility:

1. Host code (Git)
2. Collaborate (channels, PRs, issues, media)
3. Understand who contributes and how (profiles / activity blend)
4. Spin compute (Firecracker / Cloud Hypervisor / Kata on bare metal)
5. Deploy from git
6. Secrets without `.env` (Scarlet Letter ✉ + Pearl)
7. (Roadmap) Rooms, Iggy fanout, managed databases, hardened org RBAC, billing depth

**Not** a Nostr/fediverse-style product. Centralized, coherent, low-level Rust control plane with separation of concerns that *mimics* microservices inside one deployable.

**Clients (planned):** Dioxus → desktop, web, mobile over gRPC / gRPC-Web.

**Aesthetic:** ivory `#fefce5`, mint `#c4ebcc`, grey `#707070` — custom fonts and SVG icons.

---

## 2. What Exists Today

### 2.1 Identity & auth (`tt-auth`)

| Piece | Implementation |
|-------|----------------|
| Sessions | **PASETO v4 local** tokens |
| Passkeys | **webauthn-rs** (Redis ceremony store) |
| Principals | Humans and agents as first-class `Principal` |
| Agent tokens | Separate TTL for automation |
| Store | Postgres via **sqlx** (`AuthStore`) |

Flow: register/login → WebAuthn or password → PASETO bearer → `PrincipalResolver` → `AuthContext`.

### 2.2 Core domain (`tt-core`)

Shared types and IDs: workspaces, channels, messages, attachments, repositories, PRs, issues, labels, reviews, orgs, seats, compute (nodes, workloads, instances, lifecycle), profiles / contribution kinds, media metadata shapes.

### 2.3 Git hosting (`tt-git` + Smart HTTP)

- Repos under `GIT_REPOS_ROOT`
- **Smart HTTP** `…/git/{workspace}/{repo}/…`
- Real **worktree merges** (not metadata-only)
- Merge strategies + **enforceable single merge method**
- **PR protections** (approvals, up-to-date, admin-only)
- Issues, labels, **`Fixes #N` on merge**, PR reviews
- Push analysis → contribution events
- **Pre-receive / push protection** — secret-shaped string scan (`tt-secrets` patterns)
- Deploy tokens (`tt_deploy_…`) for private clone over Basic auth

### 2.4 Messaging (`tt-messaging`)

- Workspaces, channels, messages, threads, reactions
- Optional Meilisearch index on post
- **Attachments** via media IDs on send (`attachment_ids`)

### 2.5 Rooms (`tt-messaging` rooms)

Public/discoverable community feeds (`#Rust`, `#AI`, …). Roomies join with relaxed rules on public rooms. Posts: text, voice note, short clip, attachment. Live video deferred. Iggy later for fanout.

See [ROOMS.md](./ROOMS.md).

### 2.6 Orgs, seats, billing hooks (`tt-messaging` + Stripe config)

- Orgs, members, invitations, seat status
- Org policies cascade to repo settings
- SSO config surface (provider metadata)
- Stripe webhook / price id config for seat billing path

### 2.6 Profiles & activity blend (`tt-storage`)

Transparency over vanity heatmaps:

- Where someone contributes, tools, density, times
- Contribution events → Postgres + optional ClickHouse
- Profile GET/PATCH APIs for maintainers’ quick risk glance

### 2.7 Search (`tt-search` + Meilisearch)

Indexes: repos, issues, PRs, people, messages  
Tenant filter: only workspaces the caller can access  
Reindex job from Postgres; index on profile upsert / channel post

### 2.8 Media (`tt-storage` media + object store)

| Piece | Detail |
|-------|--------|
| Trait | `ObjectStore` — `put` / `get` / `delete` / `exists` |
| Local | `LocalObjectStore` under `TT_MEDIA_ROOT` |
| S3/RustFS | `S3ObjectStore` — SigV4 path-style (no aws-sdk) |
| Avatars | `POST /api/me/avatar` → `principals.avatar_url` |
| Attachments | `POST /api/workspaces/:id/media` |
| Serve | `GET /api/media/:id`, `GET /api/media/:id/thumb` |
| Thumbs | jpeg/png/webp/gif → max-edge JPEG |
| Virus scan | ClamAV `clamd` INSTREAM when `TT_CLAMD_ADDR` set; else skipped |
| Limits | Avatar 2 MiB · Attachment 25 MiB |

Migrations: `020_media`, `021_media_thumbs_scan`.

### 2.9 Scarlet Letter ✉ (`tt-secrets` + `tt-kms`)

Org / environment / repo secrets. **No plaintext on disk. Pearl never sees values.**

| Layer | Role |
|-------|------|
| Envelope | Random DEK per secret → AES-GCM data; DEK wrapped by KEK |
| KEK | **In-house `tt-kms`** on TT bare metal (not AWS/Vault as product path) |
| Providers | `tt` (production), `local` (lab), `http` (optional generic) |
| Pearl | `scarlet_probe` / list metadata only; on-device **redaction** before model context |
| CLI model | `tt secrets pull` / `tt secrets run -- cmd` → process env only |
| Push protection | Pattern scan on push + `/api/secrets/scan` for pre-commit |

See [SCARLET_LETTER.md](./SCARLET_LETTER.md) and [MEDIA.md](./MEDIA.md).

### 2.10 Agents — Pearl (`tt-agents`)

- Harness for **OLMo-3-7B** (OpenAI-compatible inference URL)
- Structured JSON tool calls, approval gates for dangerous tools
- Tools: search, git, lifecycle, compute, Scarlet meta
- **On-device redaction** of secret-shaped content before every inference call
- Step logs store sanitized args/results

### 2.11 Billing (`tt-billing`)

Dual-track: **seats** (collab) + **usage meters** (compute, git/media storage, managed Postgres/EmberDB/RustFS/Greptime). Catalog undercuts GH+Slack sum; compute never unlimited on seats. See [BILLING.md](./BILLING.md).

### 2.12 Compute (`tt-compute`, `tt-net`, `tt-node`)

Bare metal economics (Latitude / OVH / Hetzner-class):

| Piece | Detail |
|-------|--------|
| Hypervisors | Firecracker (serverless path), Cloud Hypervisor, Kata-ready |
| Node agent | `tt-node` — register, heartbeat, capacity math (overcommit) |
| Networking | Bridge/TAP, NAT port blocks (`tt-net`) |
| Deploy-from-git | Rootfs bake + cloud-init; deploy tokens |
| Quotas | Org `max_workloads`, `max_vcpu_total`, `max_concurrent_running` → 409 on exceed |
| Lifecycle | `node_ready`, `nat_allocated`, `quota_exceeded`, `instance_failed`, … |
| Orchestrator | Reconciliation loop |
| Security | Platform operator allowlist; HMAC CP↔node; encrypted deploy tokens at rest |

### 2.12 Server (`tt-server`)

Modular monolith: Axum HTTP, Smart HTTP git, sqlx migrations on boot, Redis, optional ClickHouse & Meilisearch, object store selection, Scarlet + KMS, media, agents, compute routes.

Dev seed: `POST /api/dev/bootstrap`.

---

## 3. Crate map

```
tt-server          binary — HTTP, bootstrap, routes, orchestrator spawn
tt-core            domain + IDs
tt-config          Figment / env (TT_*)
tt-auth            PASETO, WebAuthn, stores
tt-git             repos, PRs, issues, reviews, Smart HTTP, push hooks
tt-messaging       channels, orgs, workspaces
tt-compute         workloads, quotas, lifecycle, security, orchestrator
tt-net             NAT / subnet allocator
tt-node            metal agent (FC / CH / Kata drivers)
tt-storage         profiles, media, object store, contribution + lifecycle dual-write
tt-search          Meilisearch client + tenant search
tt-secrets         Scarlet Letter envelope + scan + redaction
tt-kms             In-house KMS binary (KEK on TT metal)
tt-agents          Pearl harness + tools + redaction
tt-realtime        pub/sub (Redis → Iggy later)
tt-api             shared API types
tt-audit           audit trail helpers
tt-notifications   notification stubs
tt-workflows       workflow stubs
tt-billing         seats + usage meters + managed data pricing
tt-common          shared utilities
```

---

## 4. Data plane

| Store | Role |
|-------|------|
| **PostgreSQL** | Source of truth (all domain tables + migrations 001–021) |
| **Redis** | WebAuthn ceremonies, realtime fanout (v1) |
| **ClickHouse** | Long-window contribution + lifecycle analytics (optional dual-write) |
| **Meilisearch** | Full-text search with tenant filters |
| **Object store** | Avatars + attachments (local or RustFS/S3) |
| **Git bare repos** | `GIT_REPOS_ROOT` |
| **tt-kms sealed file** | Master KEK at rest on KMS node only |

---

## 5. Security principles

1. **Tenant isolation** — membership-checked; cross-tenant → 404 not 403 where it leaks existence  
2. **Platform operators** — node registration allowlist  
3. **CP ↔ node** — HMAC timestamps; agent token hashed + encrypted at rest  
4. **Secrets** — envelope encryption; KEK in `tt-kms`; Pearl redaction; push protection  
5. **Media** — virus scan before store when clamd configured; type + size limits  
6. **Agents** — approval gates; no secret values in model context  

---

## 6. Runtime topology (production sketch)

```
                  ┌──────────── Dioxus clients ────────────┐
                  │  desktop · web · mobile (gRPC-Web)     │
                  └──────────────────┬─────────────────────┘
                                     │
              ┌──────────────────────▼──────────────────────┐
              │              tt-server (control plane)       │
              │  auth · git · msg · media · scarlet · agents │
              │  compute API · search · orgs                 │
              └─┬─────────┬──────────┬──────────┬───────────┘
                │         │          │          │
         Postgres      Redis    Meilisearch  ClickHouse
                │
         Object store (RustFS on metal)
                │
         ┌──────┴──────┐
         │   tt-kms    │  private NIC — KEK never leaves
         └─────────────┘
                │
         ┌──────┴──────────────────┐
         │  tt-node × N (bare metal)│  Firecracker / CH / Kata
         └─────────────────────────┘
```

---

## 7. Environment (essentials)

```bash
export DATABASE_URL=postgres://…/turing_tarpit
export REDIS_URL=redis://127.0.0.1:6379
export TT_SESSION_KEY=$(openssl rand -hex 32)
export TT_SECRETS_KEY=$(openssl rand -hex 32)
export TT_GIT_REPOS_ROOT=$HOME/tt-data/repos
export TT_MEDIA_ROOT=$HOME/tt-data/media
export TT_OBJECT_STORE=local          # or rustfs
export TT_KMS_PROVIDER=local          # production: tt + tt-kms
# export TT_CLAMD_ADDR=127.0.0.1:3310
# export TT_INFERENCE_URL=http://gpu-host:8000
# export TT_PLATFORM_OPERATOR_IDS=<uuid>,…
```

See [runbooks/termux-lab.md](../runbooks/termux-lab.md).

---

## 8. Migrations (Postgres)

| # | Topic |
|---|--------|
| 001 | Principals, workspaces, channels, messages, repos |
| 002–004 | PRs, repo settings, protections |
| 005 | Profiles |
| 006–007 | Issues, labels, reviews |
| 008–009 | Orgs, policies, SSO, billing |
| 010–015 | Compute, deploy tokens, lifecycle, node security |
| 016 | Agent runs |
| 017–019 | Scarlet Letter + push protection |
| 020–021 | Media + thumbs + scan status |

---

## 9. Roadmap (explicitly not done)

- Dioxus clients (desktop / web / mobile)
- **Rooms** (video) + Roomies; richer channel media UX
- Apache **Iggy** fanout when Redis pub/sub is the bottleneck
- Hardened org **RBAC** + strict human approval for agentic dangerous actions
- Billing depth refinements (SKU matrix, COGS ledger automation)
- Managed databases as a product surface
- mTLS CP ↔ tt-kms / tt-node (HMAC today)
- Argon2 for KMS unseal (SHA-256 lab derivation today)
- Full gRPC surface parity with HTTP

---

## 10. Design stance

Backend-first until the control plane is enterprise-solid. Self-host + own metal. Prefer in-house sensitive paths (KMS, compute) over competitor uptime. Agents are peers under the same permission and audit model — not a side chatbot bolted on.

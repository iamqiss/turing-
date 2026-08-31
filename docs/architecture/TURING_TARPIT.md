# Turing Tarpit — Architecture & Progress

> **Ironic name, serious product.** A self-hostable, AI-native Developer OS: code hosting, collaboration, compute, and lifecycle transparency — built as a modular Rust monolith, proprietary, backend-first.

**Last updated:** 2026-08-31  
**Status:** Control plane + compute agent functional; Dioxus clients and full product surface still ahead.

---

## 1. Vision

Turing Tarpit aims to cover the **full developer lifecycle** so humans *and* peer agents share the same visibility:

1. Host code (Git)
2. Collaborate (channels, PRs, issues)
3. Understand who is contributing and how (profiles / activity blend)
4. Spin compute (Firecracker / Cloud Hypervisor / Kata on bare metal)
5. Deploy from git
6. (Roadmap) Secrets, media Rooms, billing, agent permissions, managed data

**Not** a Nostr/fediverse-style refactor of Buzz. Centralized, coherent, low-level Rust control plane with separation of concerns that *mimics* microservices inside one deployable binary.

**Product surfaces (planned clients):** Dioxus → **desktop, web, mobile** over gRPC / gRPC-Web.

**Aesthetic:** three-colour palette — ivory, mint, grey; custom SVG icons and fonts (assets owned by TT).

---

## 2. What Exists Today

### 2.1 Identity & auth (`tt-auth`)

| Piece | Implementation |
|-------|----------------|
| Sessions | **PASETO v4 local** tokens |
| Passkeys | **webauthn-rs** ceremonies (Redis ceremony store) |
| Principals | Humans and agents as first-class `Principal` |
| Agent tokens | Separate TTL path for automation clients |
| Store | Postgres via **sqlx** (`AuthStore`) |

**Flow:** register / login → ceremony (WebAuthn) or password path → PASETO bearer on API calls → `PrincipalResolver` resolves bearer to `AuthContext`.

### 2.2 Core domain (`tt-core`)

Shared types and IDs (UUID v7-style helpers): workspaces, channels, messages, repositories, PRs, issues, labels, reviews, orgs, seats, **compute** (nodes, workloads, instances, regions, lifecycle events), profiles / contribution kinds.

### 2.3 Git hosting (`tt-git` + Smart HTTP)

- Repos on disk under `GIT_REPOS_ROOT` / workspace paths
- **Smart HTTP**: `GET/POST /git/{workspace_id}/{repo_name}/...`
- Real **worktree merges** (not metadata-only)
- PR merge strategies + **repo settings** (enforce single merge method)
- **PR protections** (reviews, up-to-date, admin-only merge flags at org policy level)
- Issues, labels, **`Fixes #N` on merge**, PR reviews
- Push analysis → contribution events (languages, density signals)

**Clone URL shape:**  
`{public_base}/git/{workspace_id}/{repo_name}`  
(`public_base` from `webauthn_rp_origin` today.)

**Private clone from compute:** short-lived **deploy tokens**  
`tt_deploy_{prefix}_{secret}` — accepted as Bearer or git HTTP Basic (`x-access-token`); hashed (+ optional AES envelope) at rest.

### 2.4 Messaging (`tt-messaging`)

Workspace/channel/message model, membership checks, reactions foundation. Realtime path via `tt-realtime` (Redis). Fanout scale-out (**Apache Iggy** preferred over Kafka) is **designed for**, not fully wired.

### 2.5 Orgs & commercial spine (`tt-messaging` org module + migrations 008–009)

- Orgs, members, roles (`owner` / `admin` / `member` / `billing`), seats
- Invitations, seat limits
- Org policy cascade (merge methods, required approvals, etc.)
- SSO config scaffold
- Stripe webhook route + customer link scaffold
- **Active member** required for compute mutations; suspended seats blocked

### 2.6 Profiles & transparency (`tt-storage` + ClickHouse)

Inspired by “Allura-level” transparency, **without** commit heatmaps:

- `contribution_events` in **Postgres** (source of truth for recent windows)
- Dual-write to **ClickHouse** `contribution_events` for long retention
- **Activity blend**: human interactive vs assisted/agent vs scripted/CI percentages, thin-activity flag, language/tool weights, density summary
- Profile cards for maintainers before trusting a large PR

Kinds include push, commit, PR/issue/review, login, agent_run, etc.

### 2.7 Turing Tarpit Cloud — compute (`tt-compute`, `tt-net`, `tt-node`)

**Economics posture (from CloudRat-class math):** bare metal (OVH / Latitude / Hetzner), overcommit, shared NAT port blocks, IPv6, quarantine cores — control plane and hypervisors in the Rust ecosystem.

| Component | Role |
|-----------|------|
| **Control plane** (`tt-server` + `tt-compute`) | Nodes, workloads, quotas, schedule, NAT allocation, lifecycle, deploy API |
| **`tt-net`** | Shared NAT IPv4 port ranges (~10 customers / IP, host ports reserved), soft subnet registry |
| **`tt-node`** | Agent on metal: Firecracker, Cloud Hypervisor (disks), Kata stub |
| **Orchestrator** | Background reconcile: stale heartbeats → offline; stuck schedules → lifecycle failure |

**Workload shapes:** `serverless` → Firecracker, `vm` → Cloud Hypervisor, `kata` → Kata.

**Start path:** quota check → pick ready node in region → allocate NAT → insert instance → HMAC-signed HTTP command to `tt-node` → TAP + bridge + optional DNAT + cloud-init / CIDATA ISO + optional git clone + rootfs bake.

**Quotas (per org):** `max_workloads`, `max_vcpu_total`, `max_concurrent_running` — enforced on create/start; `GET/PATCH /api/orgs/:slug/compute-quota`.

### 2.8 Lifecycle events (agent-visible trail)

Postgres `lifecycle_events` + ClickHouse mirror. Kinds include:

`deploy_requested`, `workload_created|started|stopped`, `nat_allocated|released`, `instance_scheduled|booted|failed`, `node_registered|ready|offline`, `quota_exceeded`, `provider_dispatch_failed`, …

- Org trail: `GET /api/orgs/:slug/lifecycle-events`
- Platform (nodes): `GET /api/compute/lifecycle-events`

### 2.9 Security model (multi-tenant)

| Control | Behavior |
|---------|----------|
| Auth | PASETO bearer on APIs |
| Org isolation | Workload start/stop/get requires **active** membership; cross-tenant IDOR → **404** |
| Platform operators | Only allowlisted principals register nodes (`TT_PLATFORM_OPERATOR_IDS` + `platform_operators` table) |
| Node agents | One-time `agent_token` at register; hash + **AES-GCM** ciphertext at rest; required on heartbeat |
| CP ↔ node | **HMAC-SHA256** over `{ts}.{body}`; headers `x-tt-timestamp`, `x-tt-signature` |
| Deploy tokens | Hash + optional encrypted blob; short TTL; read-only git |
| Secrets key | `TT_SECRETS_KEY` (32-byte hex) or session key / dev fallback |

**Principle:** never trust client-supplied org scope without membership; never reveal foreign resources exist.

### 2.10 Server binary (`tt-server`)

Modular monolith: Axum HTTP (+ Smart HTTP git, messaging routers), sqlx Postgres, Redis, optional ClickHouse, Meilisearch config hooks, S3-compatible storage config (RustFS-oriented), bootstrap + migration runner.

Dev bootstrap: `POST /api/dev/bootstrap` for local principal / org / workspace seed.

---

## 3. Crate map

```
tt-server          binary — HTTP, bootstrap, routes, orchestrator spawn
tt-core            domain + IDs
tt-config          Figment / env (TT_*)
tt-auth            PASETO, WebAuthn, stores
tt-git             repos, PRs, issues, reviews, Smart HTTP, push analytics
tt-messaging       channels, orgs, workspaces
tt-compute         workloads, quotas, lifecycle emit, security, orchestrator
tt-net             NAT / subnet allocator
tt-node            metal agent (FC / CH / Kata drivers)
tt-storage         profiles, contribution + lifecycle dual-write, ClickHouse client
tt-realtime        pub/sub (Redis)
tt-api             shared API error/types (where used)
tt-search          Meilisearch-oriented (scaffold)
tt-workflows       future CI-ish (scaffold)
tt-agents          future agent runtime policy (scaffold)
tt-audit           audit / CH hooks (scaffold)
tt-notifications   notify paths (scaffold)
tt-common          shared helpers
```

**Design rule:** crates stay separable; deploy is still one binary until fanout forces Iggy/Kafka workers.

---

## 4. How pieces interact

```text
                    ┌─────────────────────────────────────────┐
                    │              Dioxus clients               │
                    │         (web / desktop / mobile)          │
                    │              [not built yet]              │
                    └──────────────────┬──────────────────────┘
                                       │ HTTPS + gRPC-Web (planned)
                                       ▼
┌──────────────────────────────────────────────────────────────────┐
│                         tt-server (Axum)                          │
│  Auth · Git Smart HTTP · Orgs · Profiles · Compute · Messaging   │
│                         Orchestrator loop                          │
└───────┬─────────────┬──────────────┬───────────────┬─────────────┘
        │             │              │               │
        ▼             ▼              ▼               ▼
   Postgres        Redis        ClickHouse        tt-node(s)
   (OLTP)       (sessions,       (analytics)     Firecracker /
                 realtime)                        Cloud HV / Kata
        │                                              │
        └──────── GIT_REPOS_ROOT ──────────────────────┘
                  deploy clone via Smart HTTP + deploy token
```

**Deploy-from-git:**  
API → workload + lifecycle `deploy_requested` → start → NAT + signed boot → node clones `{base}/git/...` with deploy token → bake `/app` into rootfs → boot.

**Transparency loop:**  
Git push → push observer → contribution events → profile blend; compute actions → lifecycle events → same dual-write pattern for agents/ops.

---

## 5. How operators interact with it

### 5.1 Local / Termux-style lab

```bash
# Infra
# Postgres + Redis (+ optional ClickHouse)

export TT_PLATFORM_OPERATOR_IDS=<your-principal-uuid>
export TT_SECRETS_KEY=$(openssl rand -hex 32)
export CARGO_BUILD_JOBS=1   # if RAM-constrained

cargo run -p tt-server

# Bootstrap identity
curl -s -X POST http://127.0.0.1:8080/api/dev/bootstrap -H 'Content-Type: application/json' -d '{}'
# export TOKEN=...

# Register node (operator only) — save agent_token once
curl -s -X POST http://127.0.0.1:8080/api/compute/nodes \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"lat-ewr-1","region":"us-east","physical_cores":32,"memory_mb":131072,"reserved_cores":3,"overcommit_ratio":4.0}'

# Heartbeat
curl -s -X POST http://127.0.0.1:8080/api/compute/nodes/$NODE_ID/heartbeat \
  -H 'Content-Type: application/json' \
  -d '{"endpoint":"http://127.0.0.1:9090","agent_token":"tt_node_..."}'

# Shared NAT IP
curl -s -X POST http://127.0.0.1:8080/api/compute/networks/shared-nat \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"region":"us-east","ip":"203.0.113.10"}'

# Metal agent
export TT_NODE_AGENT_TOKEN=tt_node_...
cargo run -p tt-node -- --data-dir /var/lib/tt-node --bind 0.0.0.0:9090 --region us-east
```

### 5.2 Product API themes

| Area | Examples |
|------|----------|
| Git | create repo, Smart HTTP clone/push, PRs, merge, protections |
| Issues | labels, reviews, Fixes #N |
| Orgs | members, seats, policy, SSO scaffold, billing hooks |
| Profiles | GET profile card + blend |
| Compute | workloads, start/stop, deploy, quotas, lifecycle |
| Platform | operators, node register, platform lifecycle |

---

## 6. Data stores

| Store | Use |
|-------|-----|
| **Postgres** | Source of truth: auth, git metadata, orgs, workloads, quotas, events (recent) |
| **Redis** | WebAuthn ceremonies, realtime fanout, session-ish helpers |
| **ClickHouse** | Contribution + lifecycle analytics (24-month class TTL) |
| **Meilisearch** | Search (configured; product UI later) |
| **S3 / RustFS** | Attachments / objects (config ready) |
| **Disk** | Bare Git repos, `tt-node` instance dirs, images (`vmlinux`, `*.ext4`) |

Migrations: `migrations/postgres/001` … `015`; ClickHouse `001`–`003` under `migrations/clickhouse/`.

---

## 7. Roadmap — where we’re headed

### 7.1 Near-term backend

- [ ] **Platform operator** UX + bootstrap invite of first operator without env-only path
- [ ] Stronger node auth (mTLS or challenge-response beyond HMAC)
- [ ] Full **iptables/nftables** automation + bridge DHCP/cloud-init guest IP reliability
- [ ] Resolve `tt-git://` mirrors when nodes cannot reach public origin
- [ ] Managed **Postgres/Redis** product surface (launch DB alongside compute)
- [ ] Org RBAC **hardened** (resource-scoped permissions, not only org roles)

### 7.2 Messaging scale & media

- [ ] **Apache Iggy** (or Redpanda) when Redis fanout saturates — architecture already “plug in here”
- [ ] Channel **media sharing**
- [ ] **Rooms** (TT’s huddle analogue) — participants = **Roomies**; voice/video later (deferred from early scope; product voice still deferred until intentional)

### 7.3 Scarlett Letter ✉

Turing Tarpit’s own **secure environment variables and secrets sharing**:

- Encrypted secret stores per org/repo/workload
- Injection into deploy/boot without printing to logs
- Audit trail of secret access (who/what/when) for humans and agents
- Aligns with existing AES-GCM `SecretBox` + lifecycle events

### 7.4 Agentic safety

- [ ] **Strict human permissions** for dangerous agent actions (destroy prod, widen NAT, exfil repos)
- [ ] Policy engine: agent may propose; human (or dual-control) must approve
- [ ] Agent principals already exist — bind to capability grants + lifecycle evidence

### 7.5 Billing & platform admin

- [ ] Stripe seats ↔ `org_compute_quotas` and seat_limit enforcement
- [ ] Platform admin **finance console** (MRR, metal cost, margin — CloudRat-style)
- [ ] Customer invoices vs OVH/Latitude/Hetzner COGS

### 7.6 Hypervisor depth

- [ ] Production Firecracker networking + jailer
- [ ] Cloud Hypervisor persistent volumes + snapshots
- [ ] Kata for container-shaped isolation
- [ ] GPU / larger shapes when metal allows

### 7.7 Clients — AI-Native Developer OS

- [ ] **Dioxus** app shells (web, desktop, mobile)
- [ ] gRPC + gRPC-Web API surface mirroring HTTP
- [ ] Same lifecycle + blend widgets agents and humans share
- [ ] End state: one OS-like surface for code, chat, Rooms, compute, secrets, and agent supervision

---

## 8. Explicit non-goals (for now)

- Open-sourcing the product (fully proprietary)
- Federated social / Nostr-style architecture
- Premature Kafka (Iggy first if streams are needed)
- Commit-count vanity heatmaps

---

## 9. Configuration cheatsheet

| Variable | Purpose |
|----------|---------|
| `DATABASE_URL` | Postgres |
| `REDIS_URL` | Redis |
| `TT_SECRETS_KEY` | 32-byte hex AES key for tokens at rest |
| `TT_PLATFORM_OPERATOR_IDS` | Comma-separated principal UUIDs for node register |
| `GIT_REPOS_ROOT` | Bare repo storage |
| `TT_NODE_AGENT_TOKEN` | On metal; matches register response |
| `TT_NODE_DATA` | Instance/image root for tt-node |
| `TT_BRIDGE` / `TT_BRIDGE_ADDR` | TAP bridge |
| `TT_FIRECRACKER_BIN` / `TT_CH_BIN` | Hypervisor binaries |
| `TT_FC_KERNEL` / images `*.ext4` | Guest boot assets |
| ClickHouse URL/DB | Dual-write analytics |

---

## 10. Principles that stuck

1. **Backend solid before Dioxus** — enterprise control plane first.  
2. **Real Git, real merges** — not fake metadata.  
3. **Transparency over theatre** — blend + lifecycle, not heatmaps.  
4. **Tenant isolation by default** — 404 on cross-org probes; operators for metal.  
5. **Bare-metal honest economics** — overcommit, NAT, sellable cores.  
6. **Agents are principals** — same audit spine as humans; dangerous paths need humans.  
7. **Monolith with microservice seams** — crates and traits ready to split when load demands.

---

*Turing Tarpit — complete the loop, then let the Roomies in.*

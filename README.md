# Turing Tarpit

**Proprietary** self-hosted workspace for humans and AI agents.

A clean-room, centralized alternative to Slack + GitHub, built for teams that want agents as first-class collaborators without protocol tax.

## Vision

- Humans and agents share the same channels, permissions model, and audit trail.
- Built-in Git hosting so conversation and code live in one place.
- Fully self-hosted. Single binary + Compose for the happy path.
- Designed as a modular monolith so the realtime layer can later swap Redis → Apache Iggy (or Kafka) without rewriting the product.

## Stack

| Layer            | Choice                          |
|------------------|---------------------------------|
| Language         | Rust                            |
| Frontend         | Dioxus (desktop + web + mobile) |
| API              | gRPC + gRPC-Web                 |
| Primary DB       | PostgreSQL                      |
| Cache / PubSub   | Redis (→ Apache Iggy later)     |
| Analytics        | ClickHouse                      |
| Search           | Meilisearch                     |
| Git              | Gitoxide + Smart HTTP           |
| Object storage   | RustFS / S3-compatible          |
| Auth             | WebAuthn + agent API tokens     |

## Color palette

| Name  | Hex       | Role                          |
|-------|-----------|-------------------------------|
| Grey  | `#707070` | Text, borders, icons, chrome  |
| Ivory | `#fefce5` | Backgrounds, surfaces, cards  |
| Mint  | `#c4ebcc` | Accents, active, CTAs         |

Custom fonts and SVG icon set ship with the product. No external font CDNs.

## Workspace layout

```
turing-tarpit/
├── crates/           # Backend modular monolith
│   ├── tt-core       # Domain types, IDs, errors, events
│   ├── tt-common     # Shared utilities
│   ├── tt-config     # Configuration loading
│   ├── tt-auth       # WebAuthn, sessions, agent tokens
│   ├── tt-messaging  # Channels, messages, threads, DMs...
│   ├── tt-git        # Repos, Smart HTTP, PRs, reviews
│   ├── tt-search     # Meilisearch integration
│   ├── tt-storage    # Media & blob storage
│   ├── tt-realtime   # Fanout abstraction (Redis → Iggy)
│   ├── tt-agents     # Agent principals & invocation
│   ├── tt-audit      # Append-only audit log
│   ├── tt-workflows  # Automation engine (future)
│   ├── tt-notifications
│   ├── tt-api        # gRPC service implementations
│   └── tt-server     # Binary that wires everything
├── clients/
│   ├── desktop       # Dioxus desktop
│   ├── web           # Dioxus web (gRPC-Web)
│   ├── mobile        # Dioxus mobile
│   └── shared        # UI components, theme, icons, fonts
├── proto/            # Protocol Buffers
├── migrations/       # Postgres + ClickHouse
├── assets/           # Fonts, SVGs, images
├── config/           # TOML configs
├── deploy/           # Docker, k8s, systemd
└── docs/
```

## Current status

v0.1 scaffold. Most modules are empty placeholders awaiting implementation.

### v1 scope (target)

- Workspaces, channels, threads, DMs
- Messages, reactions, mentions, presence
- File uploads
- WebAuthn humans + API-token agents
- Basic Git hosting (create, push/pull, simple PR list)
- Full-text search
- Audit log
- Self-host via Docker Compose

Deferred: voice/huddles, complex workflows, multi-tenant SaaS.

## License

Proprietary. All rights reserved.


# Architecture overview

Turing Tarpit is a **modular monolith**: one primary binary (`tt-server`) with crates that own clear domains. Swap adapters (object store, KMS provider, realtime backend) without rewriting product logic.

```
Clients (CMP / any) ──gRPC · gRPC-Web · HTTP──► tt-server
                                         │
            ┌────────────────────────────┼────────────────────────────┐
            ▼              ▼             ▼              ▼             ▼
        tt-auth        tt-git      tt-messaging    tt-compute     tt-secrets
            │              │             │              │             │
            └────────── tt-core IDs & domain ───────────┴─────────────┘
                                         │
                    Postgres · Redis · Meili · CH · Object store · tt-kms
                                         │
                                    tt-node (metal)
```

| Concern | Crate(s) |
|---------|----------|
| Auth | `tt-auth` |
| Git / PR / issues | `tt-git` |
| Chat / orgs | `tt-messaging` |
| Profiles / media / analytics | `tt-storage` |
| Search | `tt-search` |
| Compute | `tt-compute`, `tt-net`, `tt-node` |
| Secrets | `tt-secrets`, `tt-kms` |
| Agents | `tt-agents` |
| Config / HTTP entry | `tt-config`, `tt-server` |

Deep dives: [TURING_TARPIT.md](./TURING_TARPIT.md) · [SCARLET_LETTER.md](./SCARLET_LETTER.md) · [MEDIA.md](./MEDIA.md)

# Testing

Prefer **unit tests** that need no Postgres/Redis. Integration/e2e under `tests/` when services are up.

## Quick

```bash
# High-value pure suites
cargo test -p tt-core
cargo test -p tt-messaging rooms::
cargo test -p tt-secrets
cargo test -p tt-compute
cargo test -p tt-storage
cargo test -p tt-kms
cargo test -p tt-agents
cargo test -p tt-net
cargo test -p tt-git hooks::

# Broader (may skip or need features)
cargo test --workspace --lib
```

## Coverage by area

| Crate | Focus |
|-------|--------|
| `tt-core` | ID roundtrip, Room serde |
| `tt-messaging` | Rooms slug/ACL/post/feed (in-memory store) |
| `tt-secrets` | Scan rules, redaction, local KMS, envelope seal |
| `tt-compute` | SecretBox, HMAC verify, org isolation helpers |
| `tt-storage` | Local object store, thumbnails |
| `tt-kms` | Seal/unseal, engine wrap after unlock |
| `tt-agents` | Context redaction, tool arg sanitization |
| `tt-net` | NAT port block math (CloudRat model) |
| `tt-git` | Push delete-ref scan short-circuit |

## Termux

```bash
export CARGO_BUILD_JOBS=1
cargo test -p tt-messaging rooms:: -- --test-threads=1
```

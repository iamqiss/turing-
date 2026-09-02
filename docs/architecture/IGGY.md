# Apache Iggy integration (Turing Tarpit)

## Enable / disable

| Mode | Config |
|------|--------|
| **Off (default)** | `realtime_backend = "memory"` |
| **On** | `realtime_backend = "iggy"` + reachable HTTP API |

Env (Figment `TT_` prefix / mapped keys):

```bash
export TT_REALTIME_BACKEND=iggy
export TT_IGGY_URL=http://127.0.0.1:3000
export TT_IGGY_USERNAME=iggy
export TT_IGGY_PASSWORD=iggy
export TT_IGGY_STREAM=tt
export TT_IGGY_TOPIC=events
```

If Iggy is unreachable at boot, TT **falls back to memory** and logs a warning — messaging still works on a single node.

## What “full” means here

1. **Dual-write** `EventPublisher`: local `InMemoryRealtime` (same-process SSE/WS) **and** Iggy HTTP publish.
2. **Topology**: auto `POST /streams` + `POST /streams/{stream}/topics` for `tt` / `events` (names configurable).
3. **Partitioning**: channel-scoped events use `messages_key = channel_id` for per-conversation order.
4. **Payload**: JSON `EventEnvelope` (base64 in Iggy message payload).
5. **Disable**: set backend back to `memory` — no Iggy calls.

Native `iggy` TCP/QUIC SDK can replace the HTTP client later without changing `EventPublisher` call sites.

## Hierarchy (not Kafka)

Stream → Topic → Partition. TT uses one stream (`tt`) and topic (`events`) by default; multi-tenant can move to `org-{id}` streams later.

## Run Iggy (dev)

```bash
# example — follow https://iggy.apache.org/docs/
docker run --rm -p 3000:3000 -p 8090:8090 apache/iggy:latest
# then:
export TT_REALTIME_BACKEND=iggy
cargo run -p tt-server
```

Workers that need cross-node fanout should **poll** `tt/events` with a consumer group (`tt-realtime`) using `PollingStrategy::next()`.

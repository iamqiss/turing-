CREATE TABLE IF NOT EXISTS turing_tarpit.lifecycle_events
(
    id              UUID,
    ts              DateTime64(3, 'UTC'),
    org_id          Nullable(UUID),
    principal_id    Nullable(UUID),
    kind            LowCardinality(String),
    workload_id     Nullable(UUID),
    instance_id     Nullable(UUID),
    node_id         Nullable(UUID),
    region          LowCardinality(Nullable(String)),
    metadata        String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (coalesce(org_id, toUUID('00000000-0000-0000-0000-000000000000')), ts, id)
TTL ts + INTERVAL 24 MONTH;

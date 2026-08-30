CREATE DATABASE IF NOT EXISTS turing_tarpit;

CREATE TABLE IF NOT EXISTS turing_tarpit.contribution_events
(
    id              UUID,
    ts              DateTime64(3, 'UTC'),
    principal_id    UUID,
    workspace_id    Nullable(UUID),
    repository_id   Nullable(UUID),
    kind            LowCardinality(String),
    client_kind     LowCardinality(String),
    tool_id         Nullable(String),
    languages       Array(String),
    files_touched   UInt32,
    lines_added     UInt32,
    lines_deleted   UInt32,
    is_automation   UInt8,
    metadata        String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (principal_id, ts, id)
TTL ts + INTERVAL 24 MONTH;

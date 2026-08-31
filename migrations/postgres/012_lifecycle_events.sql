-- Compute lifecycle facts (OLTP); ClickHouse mirrors for long-window analytics

CREATE TABLE IF NOT EXISTS lifecycle_events (
    id              UUID PRIMARY KEY,
    ts              TIMESTAMPTZ NOT NULL,
    org_id          UUID REFERENCES orgs(id) ON DELETE CASCADE,
    principal_id    UUID REFERENCES principals(id) ON DELETE SET NULL,
    kind            TEXT NOT NULL,
    workload_id     UUID REFERENCES workloads(id) ON DELETE SET NULL,
    instance_id     UUID REFERENCES instances(id) ON DELETE SET NULL,
    node_id         UUID REFERENCES compute_nodes(id) ON DELETE SET NULL,
    region          TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS lifecycle_events_org_ts_idx
    ON lifecycle_events (org_id, ts DESC);
CREATE INDEX IF NOT EXISTS lifecycle_events_workload_idx
    ON lifecycle_events (workload_id, ts DESC)
    WHERE workload_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS lifecycle_events_kind_idx
    ON lifecycle_events (kind, ts DESC);

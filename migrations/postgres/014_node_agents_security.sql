-- Node agent credentials: tt-node must prove identity on heartbeat / command ack
ALTER TABLE compute_nodes
    ADD COLUMN IF NOT EXISTS agent_token_hash TEXT,
    ADD COLUMN IF NOT EXISTS agent_token_prefix TEXT;

-- Platform operators (node register) — principals allowed to manage fleet
CREATE TABLE IF NOT EXISTS platform_operators (
    principal_id    UUID PRIMARY KEY REFERENCES principals(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Optional: bind workload access audit
CREATE INDEX IF NOT EXISTS workloads_org_id_idx ON workloads (org_id, id);
CREATE INDEX IF NOT EXISTS instances_workload_idx ON instances (workload_id);

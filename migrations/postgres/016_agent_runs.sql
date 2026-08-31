-- Agent task runs + pending human approvals

CREATE TABLE IF NOT EXISTS agent_runs (
    id              UUID PRIMARY KEY,
    agent_id        UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    owner_id        UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    workspace_id    UUID REFERENCES workspaces(id) ON DELETE SET NULL,
    org_id          UUID REFERENCES orgs(id) ON DELETE SET NULL,
    status          TEXT NOT NULL DEFAULT 'queued',
    goal            TEXT NOT NULL,
    model           TEXT NOT NULL DEFAULT 'olmo-3-7b',
    max_steps       INT NOT NULL DEFAULT 12,
    step_count      INT NOT NULL DEFAULT 0,
    result_summary  TEXT,
    error           TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS agent_runs_owner_idx ON agent_runs (owner_id, created_at DESC);
CREATE INDEX IF NOT EXISTS agent_runs_status_idx ON agent_runs (status, created_at DESC);

CREATE TABLE IF NOT EXISTS agent_run_steps (
    id              UUID PRIMARY KEY,
    run_id          UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    step_index      INT NOT NULL,
    role            TEXT NOT NULL, -- system|user|assistant|tool
    content         TEXT NOT NULL,
    tool_name       TEXT,
    tool_args       JSONB,
    tool_result     JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, step_index)
);

CREATE TABLE IF NOT EXISTS agent_approvals (
    id              UUID PRIMARY KEY,
    run_id          UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    tool_name       TEXT NOT NULL,
    tool_args       JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending', -- pending|approved|rejected
    requested_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at     TIMESTAMPTZ,
    resolved_by     UUID REFERENCES principals(id)
);

CREATE INDEX IF NOT EXISTS agent_approvals_pending_idx
    ON agent_approvals (status, requested_at)
    WHERE status = 'pending';

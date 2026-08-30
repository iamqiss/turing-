-- User profiles + contribution facts (OLTP; ClickHouse mirrors for analytics)

CREATE TABLE IF NOT EXISTS profiles (
    principal_id    UUID PRIMARY KEY REFERENCES principals(id) ON DELETE CASCADE,
    bio             TEXT,
    location        TEXT,
    website         TEXT,
    links           JSONB NOT NULL DEFAULT '[]'::jsonb,
    is_public       BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER profiles_updated_at
    BEFORE UPDATE ON profiles
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE IF NOT EXISTS contribution_events (
    id              UUID PRIMARY KEY,
    ts              TIMESTAMPTZ NOT NULL,
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    workspace_id    UUID REFERENCES workspaces(id) ON DELETE SET NULL,
    repository_id   UUID REFERENCES repositories(id) ON DELETE SET NULL,
    kind            TEXT NOT NULL,
    client_kind     TEXT NOT NULL DEFAULT 'unknown',
    tool_id         TEXT,
    languages       TEXT[] NOT NULL DEFAULT '{}',
    files_touched   INT NOT NULL DEFAULT 0,
    lines_added     INT NOT NULL DEFAULT 0,
    lines_deleted   INT NOT NULL DEFAULT 0,
    is_automation   BOOLEAN NOT NULL DEFAULT FALSE,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS contribution_events_principal_ts_idx
    ON contribution_events (principal_id, ts DESC);
CREATE INDEX IF NOT EXISTS contribution_events_ts_idx
    ON contribution_events (ts DESC);
CREATE INDEX IF NOT EXISTS contribution_events_repo_idx
    ON contribution_events (repository_id, ts DESC)
    WHERE repository_id IS NOT NULL;

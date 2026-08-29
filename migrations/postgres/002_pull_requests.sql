-- Pull requests (GitHub-style) + sequential numbers per repository

CREATE TABLE IF NOT EXISTS pull_requests (
    id              UUID PRIMARY KEY,
    repository_id   UUID NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    number          INT  NOT NULL,
    title           TEXT NOT NULL,
    body            TEXT,
    author_id       UUID NOT NULL REFERENCES principals(id),
    source_branch   TEXT NOT NULL,
    target_branch   TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('open', 'closed', 'merged')),
    merged_by       UUID REFERENCES principals(id),
    merged_at       TIMESTAMPTZ,
    closed_at       TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (repository_id, number)
);

CREATE INDEX IF NOT EXISTS pull_requests_repo_idx ON pull_requests (repository_id);
CREATE INDEX IF NOT EXISTS pull_requests_repo_status_idx ON pull_requests (repository_id, status);
CREATE INDEX IF NOT EXISTS pull_requests_author_idx ON pull_requests (author_id);

CREATE TRIGGER pull_requests_updated_at
    BEFORE UPDATE ON pull_requests
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Counter table for atomic PR numbers
CREATE TABLE IF NOT EXISTS repository_pr_counters (
    repository_id   UUID PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    last_number     INT NOT NULL DEFAULT 0
);

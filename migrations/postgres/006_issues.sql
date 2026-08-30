CREATE TABLE IF NOT EXISTS issues (
    id              UUID PRIMARY KEY,
    repository_id   UUID NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    number          INT  NOT NULL,
    title           TEXT NOT NULL,
    body            TEXT,
    author_id       UUID NOT NULL REFERENCES principals(id),
    state           TEXT NOT NULL CHECK (state IN ('open', 'closed')),
    closed_at       TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (repository_id, number)
);

CREATE INDEX IF NOT EXISTS issues_repo_idx ON issues (repository_id);
CREATE INDEX IF NOT EXISTS issues_repo_state_idx ON issues (repository_id, state);
CREATE INDEX IF NOT EXISTS issues_author_idx ON issues (author_id);

CREATE TRIGGER issues_updated_at
    BEFORE UPDATE ON issues
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE IF NOT EXISTS repository_issue_counters (
    repository_id   UUID PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    last_number     INT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS issue_comments (
    id              UUID PRIMARY KEY,
    issue_id        UUID NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    author_id       UUID NOT NULL REFERENCES principals(id),
    body            TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS issue_comments_issue_idx ON issue_comments (issue_id, created_at);

CREATE TRIGGER issue_comments_updated_at
    BEFORE UPDATE ON issue_comments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

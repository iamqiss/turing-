-- Labels
CREATE TABLE IF NOT EXISTS labels (
    id              UUID PRIMARY KEY,
    repository_id   UUID NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    color           TEXT NOT NULL DEFAULT '888888',
    description     TEXT,
    UNIQUE (repository_id, name)
);

CREATE INDEX IF NOT EXISTS labels_repo_idx ON labels (repository_id);

CREATE TABLE IF NOT EXISTS issue_labels (
    issue_id    UUID NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    label_id    UUID NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (issue_id, label_id)
);

-- PR reviews
CREATE TABLE IF NOT EXISTS reviews (
    id                  UUID PRIMARY KEY,
    pull_request_id     UUID NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    author_id           UUID NOT NULL REFERENCES principals(id),
    state               TEXT NOT NULL CHECK (state IN ('comment', 'approve', 'request_changes')),
    body                TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS reviews_pr_idx ON reviews (pull_request_id, created_at);

-- PR protection: required approvals
ALTER TABLE repository_settings
    ADD COLUMN IF NOT EXISTS required_approvals INT NOT NULL DEFAULT 0;

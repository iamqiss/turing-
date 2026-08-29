CREATE TABLE IF NOT EXISTS repository_settings (
    repository_id           UUID PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    allowed_merge_methods   TEXT[] NOT NULL DEFAULT ARRAY['merge','squash','rebase'],
    default_merge_method    TEXT NOT NULL DEFAULT 'merge'
        CHECK (default_merge_method IN ('merge', 'squash', 'rebase')),
    delete_branch_on_merge  BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER repository_settings_updated_at
    BEFORE UPDATE ON repository_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

ALTER TABLE repository_settings
    ADD COLUMN IF NOT EXISTS require_up_to_date BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE repository_settings
    ADD COLUMN IF NOT EXISTS restrict_merge_to_admins BOOLEAN NOT NULL DEFAULT FALSE;

-- Scarlet Letter push protection (secret-shaped string blocking)

ALTER TABLE repository_settings
    ADD COLUMN IF NOT EXISTS block_secret_push BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE repository_settings
    ADD COLUMN IF NOT EXISTS secret_push_allowlist TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];

-- Media: channel attachments + profile avatars (object storage keys)

CREATE TABLE IF NOT EXISTS media_objects (
    id              UUID PRIMARY KEY,
    owner_id        UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN ('avatar', 'attachment', 'other')),
    filename        TEXT NOT NULL,
    content_type    TEXT NOT NULL,
    size_bytes      BIGINT NOT NULL CHECK (size_bytes >= 0),
    storage_key     TEXT NOT NULL UNIQUE,
    width           INT,
    height          INT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS media_objects_owner_idx ON media_objects (owner_id, created_at DESC);

CREATE TABLE IF NOT EXISTS message_attachments (
    message_id      UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    media_id        UUID NOT NULL REFERENCES media_objects(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, media_id)
);

CREATE INDEX IF NOT EXISTS message_attachments_media_idx ON message_attachments (media_id);

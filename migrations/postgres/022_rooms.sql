-- Rooms: public/discoverable community feeds (Roomies = members)
-- Live video deferred; voice notes + short clips via media_objects.

CREATE TABLE IF NOT EXISTS rooms (
    id              UUID PRIMARY KEY,
    slug            TEXT NOT NULL,
    title           TEXT NOT NULL,
    description     TEXT,
    visibility      TEXT NOT NULL CHECK (visibility IN ('public', 'private')),
    created_by      UUID NOT NULL REFERENCES principals(id),
    member_count    INT NOT NULL DEFAULT 0,
    is_archived     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS rooms_slug_idx ON rooms (lower(slug));
CREATE INDEX IF NOT EXISTS rooms_visibility_idx ON rooms (visibility) WHERE NOT is_archived;
CREATE INDEX IF NOT EXISTS rooms_created_idx ON rooms (created_at DESC);

CREATE TABLE IF NOT EXISTS room_members (
    room_id         UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    role            TEXT NOT NULL DEFAULT 'member'
                    CHECK (role IN ('owner', 'moderator', 'member')),
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (room_id, principal_id)
);

CREATE INDEX IF NOT EXISTS room_members_principal_idx ON room_members (principal_id);

CREATE TABLE IF NOT EXISTS room_posts (
    id              UUID PRIMARY KEY,
    room_id         UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    author_id       UUID NOT NULL REFERENCES principals(id),
    kind            TEXT NOT NULL CHECK (kind IN ('text', 'voice_note', 'video_clip', 'attachment')),
    content         TEXT NOT NULL DEFAULT '',
    media_id        UUID REFERENCES media_objects(id) ON DELETE SET NULL,
    is_edited       BOOLEAN NOT NULL DEFAULT FALSE,
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS room_posts_room_created_idx ON room_posts (room_id, created_at DESC);
CREATE INDEX IF NOT EXISTS room_posts_author_idx ON room_posts (author_id);

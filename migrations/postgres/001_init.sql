-- Turing Tarpit – initial schema
-- Version: 001
-- Source of truth for workspaces, principals, channels, messages, memberships, repos.

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ---------------------------------------------------------------------------
-- Principals (humans + agents)
-- ---------------------------------------------------------------------------
CREATE TABLE principals (
    id              UUID PRIMARY KEY,
    kind            TEXT NOT NULL CHECK (kind IN ('human', 'agent')),
    display_name    TEXT NOT NULL,
    handle          TEXT NOT NULL,
    avatar_url      TEXT,
    email           TEXT,
    owner_id        UUID REFERENCES principals(id) ON DELETE SET NULL, -- agents only
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT agents_have_owner CHECK (
        (kind = 'human' AND owner_id IS NULL) OR
        (kind = 'agent' AND owner_id IS NOT NULL)
    )
);

CREATE UNIQUE INDEX principals_handle_idx ON principals (lower(handle));
CREATE UNIQUE INDEX principals_email_idx  ON principals (lower(email)) WHERE email IS NOT NULL;
CREATE INDEX principals_owner_idx ON principals (owner_id) WHERE owner_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Workspaces
-- ---------------------------------------------------------------------------
CREATE TABLE workspaces (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL,
    slug            TEXT NOT NULL,
    description     TEXT,
    owner_id        UUID NOT NULL REFERENCES principals(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX workspaces_slug_idx ON workspaces (lower(slug));

CREATE TABLE workspace_members (
    workspace_id    UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'guest')),
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, principal_id)
);

CREATE INDEX workspace_members_principal_idx ON workspace_members (principal_id);

-- ---------------------------------------------------------------------------
-- Channels
-- ---------------------------------------------------------------------------
CREATE TABLE channels (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN ('public', 'private', 'direct')),
    name            TEXT,                     -- NULL for pure DMs
    topic           TEXT,
    is_archived     BOOLEAN NOT NULL DEFAULT FALSE,
    created_by      UUID NOT NULL REFERENCES principals(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX channels_workspace_idx ON channels (workspace_id);
CREATE INDEX channels_workspace_name_idx ON channels (workspace_id, lower(name)) WHERE name IS NOT NULL;

CREATE TABLE channel_members (
    channel_id      UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_read_at    TIMESTAMPTZ,
    PRIMARY KEY (channel_id, principal_id)
);

CREATE INDEX channel_members_principal_idx ON channel_members (principal_id);

-- ---------------------------------------------------------------------------
-- Messages & threads
-- ---------------------------------------------------------------------------
CREATE TABLE threads (
    id                  UUID PRIMARY KEY,
    channel_id          UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    root_message_id     UUID,                 -- filled after root message exists
    reply_count         INT NOT NULL DEFAULT 0,
    last_reply_at       TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX threads_channel_idx ON threads (channel_id);

CREATE TABLE messages (
    id              UUID PRIMARY KEY,
    channel_id      UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    thread_id       UUID REFERENCES threads(id) ON DELETE SET NULL,
    author_id       UUID NOT NULL REFERENCES principals(id),
    content         TEXT NOT NULL,
    reply_to_id     UUID REFERENCES messages(id) ON DELETE SET NULL,
    is_edited       BOOLEAN NOT NULL DEFAULT FALSE,
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX messages_channel_created_idx ON messages (channel_id, created_at DESC);
CREATE INDEX messages_thread_idx ON messages (thread_id, created_at) WHERE thread_id IS NOT NULL;
CREATE INDEX messages_author_idx ON messages (author_id);

-- Back-reference for thread root (optional integrity)
ALTER TABLE threads
    ADD CONSTRAINT threads_root_message_fk
    FOREIGN KEY (root_message_id) REFERENCES messages(id) ON DELETE SET NULL;

CREATE TABLE reactions (
    message_id      UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    emoji           TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (message_id, principal_id, emoji)
);

CREATE TABLE attachments (
    id              UUID PRIMARY KEY,
    message_id      UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename        TEXT NOT NULL,
    content_type    TEXT NOT NULL,
    size_bytes      BIGINT NOT NULL CHECK (size_bytes >= 0),
    storage_key     TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX attachments_message_idx ON attachments (message_id);

-- ---------------------------------------------------------------------------
-- Repositories (basic Git hosting metadata)
-- ---------------------------------------------------------------------------
CREATE TABLE repositories (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT,
    default_branch  TEXT NOT NULL DEFAULT 'main',
    is_private      BOOLEAN NOT NULL DEFAULT TRUE,
    created_by      UUID NOT NULL REFERENCES principals(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workspace_id, name)
);

CREATE INDEX repositories_workspace_idx ON repositories (workspace_id);

-- ---------------------------------------------------------------------------
-- Sessions (WebAuthn / token sessions for humans)
-- ---------------------------------------------------------------------------
CREATE TABLE sessions (
    id              UUID PRIMARY KEY,
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    token_hash      TEXT NOT NULL,            -- hashed session / API token
    user_agent      TEXT,
    ip_address      TEXT,
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX sessions_principal_idx ON sessions (principal_id);
CREATE INDEX sessions_token_hash_idx ON sessions (token_hash);
CREATE INDEX sessions_expires_idx ON sessions (expires_at);

-- ---------------------------------------------------------------------------
-- Agent API tokens
-- ---------------------------------------------------------------------------
CREATE TABLE agent_tokens (
    id              UUID PRIMARY KEY,
    agent_id        UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,            -- human label
    token_hash      TEXT NOT NULL,
    scopes          TEXT[] NOT NULL DEFAULT '{}',
    expires_at      TIMESTAMPTZ,
    last_used_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ
);

CREATE INDEX agent_tokens_agent_idx ON agent_tokens (agent_id);
CREATE UNIQUE INDEX agent_tokens_hash_idx ON agent_tokens (token_hash);

-- ---------------------------------------------------------------------------
-- Audit log (lightweight, Postgres for v1; ClickHouse later for heavy analytics)
-- ---------------------------------------------------------------------------
CREATE TABLE audit_events (
    id              UUID PRIMARY KEY,
    workspace_id    UUID REFERENCES workspaces(id) ON DELETE SET NULL,
    actor_id        UUID REFERENCES principals(id) ON DELETE SET NULL,
    action          TEXT NOT NULL,
    resource_type   TEXT,
    resource_id     UUID,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX audit_events_workspace_created_idx ON audit_events (workspace_id, created_at DESC);
CREATE INDEX audit_events_actor_idx ON audit_events (actor_id, created_at DESC);

-- ---------------------------------------------------------------------------
-- Updated_at trigger helper
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER principals_updated_at
    BEFORE UPDATE ON principals
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER workspaces_updated_at
    BEFORE UPDATE ON workspaces
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER channels_updated_at
    BEFORE UPDATE ON channels
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER messages_updated_at
    BEFORE UPDATE ON messages
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER repositories_updated_at
    BEFORE UPDATE ON repositories
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- WebAuthn passkeys (serialized Passkey from webauthn-rs)
-- ---------------------------------------------------------------------------
CREATE TABLE passkeys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id    UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    -- webauthn-rs Passkey is serialized as JSON (cred_id, cose key, counter, etc.)
    credential_data JSONB NOT NULL,
    -- optional convenience columns for lookups
    cred_id         BYTEA,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX passkeys_principal_idx ON passkeys (principal_id);
CREATE UNIQUE INDEX passkeys_cred_id_idx ON passkeys (cred_id) WHERE cred_id IS NOT NULL;

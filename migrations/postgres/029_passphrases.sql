-- Human passphrases (optional alternative to WebAuthn).
-- One active passphrase credential per principal; rotated by upsert.
-- Hash format: PHC string from argon2 (e.g. $argon2id$v=19$...)

CREATE TABLE IF NOT EXISTS principal_passphrases (
    principal_id    UUID PRIMARY KEY REFERENCES principals(id) ON DELETE CASCADE,
    passphrase_hash TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER principal_passphrases_updated_at
    BEFORE UPDATE ON principal_passphrases
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();

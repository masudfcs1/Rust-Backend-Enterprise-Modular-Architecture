-- A session is a refresh-token family. Revocation also invalidates its access JWTs.
CREATE TABLE auth_sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    last_rotated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT auth_sessions_valid_expiry CHECK (expires_at > created_at)
);

CREATE INDEX auth_sessions_user_id_idx ON auth_sessions (user_id);
CREATE INDEX auth_sessions_expires_at_idx ON auth_sessions (expires_at);

-- Keep consumed hashes until the family expires so replay revokes the entire family.
-- Store SHA-256 digests only; plaintext bearer tokens never enter the database.
CREATE TABLE auth_refresh_tokens (
    token_hash BYTEA PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES auth_sessions (id) ON DELETE CASCADE,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    consumed_at TIMESTAMPTZ,
    CONSTRAINT auth_refresh_tokens_hash_length CHECK (OCTET_LENGTH(token_hash) = 32)
);

CREATE INDEX auth_refresh_tokens_session_id_idx ON auth_refresh_tokens (session_id);
CREATE UNIQUE INDEX auth_refresh_tokens_one_active_per_session_idx
    ON auth_refresh_tokens (session_id) WHERE consumed_at IS NULL;

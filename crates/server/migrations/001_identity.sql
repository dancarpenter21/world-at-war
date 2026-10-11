CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    checksum TEXT NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE players (
    id UUID PRIMARY KEY,
    display_name TEXT NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 80),
    username TEXT UNIQUE CHECK (username ~ '^[a-z0-9_]{3,40}$'),
    password_hash TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((username IS NULL) = (password_hash IS NULL))
);

CREATE TABLE browser_sessions (
    token_hash TEXT PRIMARY KEY,
    player_id UUID NOT NULL REFERENCES players(id),
    csrf_token TEXT NOT NULL,
    expires_unix BIGINT NOT NULL CHECK (expires_unix > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX browser_sessions_expiry ON browser_sessions(expires_unix);
CREATE INDEX browser_sessions_player ON browser_sessions(player_id);

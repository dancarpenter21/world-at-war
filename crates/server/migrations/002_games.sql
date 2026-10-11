CREATE TABLE games (
    id UUID PRIMARY KEY,
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    snapshot_sequence BIGINT NOT NULL,
    snapshot BYTEA NOT NULL,
    snapshot_sha256 TEXT NOT NULL,
    current_sha256 TEXT NOT NULL
);
CREATE TABLE game_events (
    game_id UUID NOT NULL REFERENCES games(id),
    sequence BIGINT NOT NULL,
    kind TEXT NOT NULL,
    actor UUID,
    operation TEXT NOT NULL,
    payload BYTEA NOT NULL,
    state_sha256 TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(game_id, sequence)
);
CREATE TABLE game_memberships (
    game_id UUID NOT NULL REFERENCES games(id),
    player_id UUID NOT NULL REFERENCES players(id),
    display_name TEXT NOT NULL,
    PRIMARY KEY(game_id, player_id)
);
CREATE TABLE game_seats (
    game_id UUID NOT NULL REFERENCES games(id),
    seat_id UUID NOT NULL,
    owner_id UUID REFERENCES players(id),
    grant_id UUID REFERENCES players(id),
    generation BIGINT NOT NULL CHECK (generation >= 0),
    lease JSONB,
    PRIMARY KEY(game_id, seat_id)
);

-- Queryable metadata remains separate from the private deterministic snapshot.
CREATE TABLE game_metadata (
    game_id UUID PRIMARY KEY REFERENCES games(id),
    title TEXT NOT NULL,
    host_id UUID NOT NULL REFERENCES players(id),
    scenario_id TEXT NOT NULL,
    scenario_version BIGINT NOT NULL,
    scenario_checksum TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('lobby', 'running', 'paused')),
    tick BIGINT NOT NULL CHECK (tick >= 0)
);

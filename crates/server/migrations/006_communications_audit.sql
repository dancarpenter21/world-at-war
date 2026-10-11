CREATE TABLE communications_audit (
    game_id UUID NOT NULL REFERENCES games(id),
    boundary BIGINT NOT NULL,
    ordinal BIGINT NOT NULL,
    message_key TEXT NOT NULL,
    detail JSONB NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (game_id, boundary, ordinal)
);
CREATE INDEX communications_audit_retention ON communications_audit(game_id, message_key, recorded_at);
CREATE TABLE communications_audit_roles (
    game_id UUID NOT NULL REFERENCES games(id),
    role_id UUID NOT NULL,
    sequence BIGINT NOT NULL,
    boundary BIGINT NOT NULL,
    ordinal BIGINT NOT NULL,
    detail JSONB NOT NULL,
    PRIMARY KEY (game_id, role_id, sequence),
    FOREIGN KEY (game_id, boundary, ordinal) REFERENCES communications_audit(game_id, boundary, ordinal) ON DELETE CASCADE
);
CREATE TABLE communications_audit_runtime (
    game_id UUID PRIMARY KEY REFERENCES games(id),
    correlations JSONB NOT NULL,
    heads JSONB NOT NULL,
    coverage_tick BIGINT NOT NULL,
    partial_history BOOLEAN NOT NULL DEFAULT false,
    floors JSONB NOT NULL DEFAULT '{}'::JSONB,
    lifecycle_floors JSONB NOT NULL DEFAULT '{}'::JSONB
);
ALTER TABLE network_events ADD COLUMN recorded_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP;
CREATE INDEX network_events_retention ON network_events(game_id, message_id, recorded_at);

-- Message content is immutable and stored once; lifecycle rows reference it.
CREATE TABLE network_messages (
    game_id UUID NOT NULL REFERENCES games(id),
    message_id UUID NOT NULL,
    profile_id TEXT NOT NULL,
    origin_role UUID NOT NULL,
    origin_entity UUID NOT NULL,
    recipient_entity UUID NOT NULL,
    content JSONB NOT NULL,
    encoded BYTEA NOT NULL,
    PRIMARY KEY(game_id, message_id)
);
CREATE TABLE network_events (
    game_id UUID NOT NULL,
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    message_id UUID NOT NULL,
    detail JSONB NOT NULL,
    PRIMARY KEY(game_id, sequence),
    FOREIGN KEY(game_id, message_id) REFERENCES network_messages(game_id, message_id)
);
CREATE INDEX network_events_message ON network_events(game_id, message_id, sequence);

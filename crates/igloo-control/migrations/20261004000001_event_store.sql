-- One row per entity: its kind and the number of events committed for it, used for optimistic
-- concurrency and for listing entities of a kind.
CREATE TABLE streams (
    subject TEXT PRIMARY KEY,
    entity TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);

CREATE INDEX streams_by_entity ON streams (entity, subject);

-- The global event log and every entity's source of truth. `sequence` is assigned under a
-- transaction-level advisory lock, so it is gapless and increases in commit order.
CREATE TABLE events (
    sequence BIGINT PRIMARY KEY CHECK (sequence > 0),
    id TEXT NOT NULL UNIQUE,
    type TEXT NOT NULL,
    schema_version SMALLINT NOT NULL,
    subject TEXT NOT NULL REFERENCES streams (subject),
    stream_version BIGINT NOT NULL CHECK (stream_version > 0),
    time TIMESTAMPTZ NOT NULL,
    actor JSONB NOT NULL,
    correlation_id TEXT NOT NULL,
    causation_id TEXT,
    data JSONB NOT NULL,
    UNIQUE (subject, stream_version)
);

-- How far each event consumer has processed the log.
CREATE TABLE checkpoints (
    consumer TEXT PRIMARY KEY,
    position BIGINT NOT NULL CHECK (position >= 0)
);

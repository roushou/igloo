-- Job output, one row per chunk. A chunk is identified by its byte offset within its stream, so
-- resent chunks are ignored. One gateway session appends a job's chunks in order.
CREATE TABLE job_logs (
    sequence BIGSERIAL PRIMARY KEY,
    job_id TEXT NOT NULL,
    stream TEXT NOT NULL CHECK (stream IN ('stdout', 'stderr')),
    byte_offset BIGINT NOT NULL CHECK (byte_offset >= 0),
    data BYTEA NOT NULL,
    UNIQUE (job_id, stream, byte_offset)
);

CREATE INDEX job_logs_by_job ON job_logs (job_id, sequence);

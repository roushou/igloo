-- Requests made with an idempotency key, per actor. `status` and `body` are set once the
-- request completed; until then the row is a claim held by the request in progress.
CREATE TABLE idempotency_keys (
    actor TEXT NOT NULL,
    key TEXT NOT NULL,
    fingerprint BYTEA NOT NULL,
    claimed_at TIMESTAMPTZ NOT NULL,
    status SMALLINT,
    body BYTEA,
    PRIMARY KEY (actor, key),
    CHECK ((status IS NULL) = (body IS NULL))
);

CREATE INDEX idempotency_keys_by_claim ON idempotency_keys (claimed_at);

-- Repository secrets, encrypted with XChaCha20-Poly1305 under the server's secrets key; the
-- repository id and name are the associated data, so a value only decrypts where it was stored.
CREATE TABLE repo_secrets (
    repo_id TEXT NOT NULL,
    name TEXT NOT NULL,
    nonce BYTEA NOT NULL CHECK (length(nonce) = 24),
    ciphertext BYTEA NOT NULL,
    PRIMARY KEY (repo_id, name)
);

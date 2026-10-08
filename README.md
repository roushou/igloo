# Igloo

A self-hosted platform where humans and agents build and maintain software: instant forks of
warm environments, safe execution of untrusted code, CI, and an agent runtime.

- Agents: start at `AGENTS.md`.
- Humans: `docs/architecture.md`, then `docs/workflow.md`.

## Run it locally

```sh
docker compose -f dev/compose.yaml up -d --wait
IGLOO_DATABASE_URL=postgres://igloo:igloo@localhost:5432/igloo IGLOO_DEV_TOKEN=dev \
  IGLOO_JOIN_TOKEN=join IGLOO_BLOB_KEY=dev-blob-key-at-least-32-bytes-long \
  IGLOO_SECRETS_KEY=dev-secrets-key-at-least-32-bytes \
  IGLOO_EMBEDDED_WORKER=true cargo run --bin igloo-control
# in any project directory:
IGLOO_TOKEN=dev cargo run --manifest-path /path/to/igloo/Cargo.toml --bin igloo -- run -- cargo test
```

The embedded worker runs commands without isolation; use it only for your own code.

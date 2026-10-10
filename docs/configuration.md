# Configuration

Every setting of every binary, with its default. Each binary reads only its own variables, so one
shell can hold all three sets. An empty value counts as unset. Problems, unknown variables
included, are reported together at startup, by variable name. Secrets are never logged.

## Server (`igloo-control`)

Reads `IGLOO_*`, except `IGLOO_WORKER_*`, `IGLOO_API` and `IGLOO_TOKEN`. An unknown `IGLOO_*`
variable stops it at startup. Once serving, it logs its addresses, data directory, public URL and
whether it embeds a worker.

| Variable                        | Default                              | Meaning                                                                                          |
| ------------------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------ |
| `IGLOO_DATABASE_URL`            | required                             | Postgres URL, `postgres://` or `postgresql://`.                                                  |
| `IGLOO_DEV_TOKEN`               | required                             | Bearer token for REST and MCP; maps to one human with every permission.                          |
| `IGLOO_JOIN_TOKEN`              | required                             | Token workers join with.                                                                         |
| `IGLOO_BLOB_KEY`                | required                             | Signs presigned blob URLs. At least 32 bytes.                                                    |
| `IGLOO_SECRETS_KEY`             | required                             | Encrypts repository secrets. At least 32 bytes. Changing it loses every secret.                  |
| `IGLOO_LISTEN`                  | `127.0.0.1:7000`                     | REST, SSE and MCP address.                                                                       |
| `IGLOO_GATEWAY_LISTEN`          | `127.0.0.1:7001`                     | Worker gateway (gRPC) address.                                                                   |
| `IGLOO_DATA_DIR`                | the user's data directory, see below | Blobs, repository mirrors, image downloads, the embedded worker's state.                         |
| `IGLOO_PUBLIC_URL`              | `http://` and the bound REST address | The REST URL as workers and sandboxes reach it: blob downloads and workspaces' git origin.       |
| `IGLOO_LEASE_TTL_SECONDS`       | `30`                                 | How long a job lease lasts without a heartbeat. 1 to 3600.                                       |
| `IGLOO_MIRROR_INTERVAL_SECONDS` | `900`                                | How often each default branch is pushed to its forge. 10 to 86400.                               |
| `IGLOO_COMMAND_TIMEOUT_SECONDS` | `10`                                 | How long one command may take. 1 to 300.                                                         |
| `IGLOO_LOG_FORMAT`              | `pretty`                             | `pretty` or `json`.                                                                              |
| `IGLOO_EMBEDDED_WORKER`         | `false`                              | Runs an unisolated worker in the server process. Development only.                               |
| `IGLOO_WEB_DIR`                 | unset                                | A built web console (`web/dist/client`, holding `index.html`), served at `/`. Unset, `/` is 404. |
| `RUST_LOG`                      | `info`                               | Log filter.                                                                                      |

`IGLOO_DATA_DIR` defaults to:

| Platform | Directory                                                          |
| -------- | ------------------------------------------------------------------ |
| macOS    | `~/Library/Application Support/igloo`                              |
| Linux    | `$XDG_DATA_HOME/igloo`, or `~/.local/share/igloo` when it is unset |

Without a home directory, `IGLOO_DATA_DIR` is required. Inside it:

| Path         | Holds                                     |
| ------------ | ----------------------------------------- |
| `blobs/`     | Layers, snapshots and uploads, by digest  |
| `downloads/` | Image layers fetched from OCI registries  |
| `repos/`     | One bare git mirror per repository        |
| `worker/`    | The embedded worker's state, when it runs |

## Worker (`igloo-worker`)

Reads `IGLOO_WORKER_*`.

| Variable                       | Default  | Meaning                                                                   |
| ------------------------------ | -------- | ------------------------------------------------------------------------- |
| `IGLOO_WORKER_SERVER`          | required | The gateway's URL, such as `http://127.0.0.1:7001`.                       |
| `IGLOO_WORKER_JOIN_TOKEN`      | required | The server's `IGLOO_JOIN_TOKEN`.                                          |
| `IGLOO_WORKER_DATA_DIR`        | required | Worker id, sandboxes, layer cache and result outbox.                      |
| `IGLOO_WORKER_RUNTIME`         | required | `oci` (Linux) or `process` (no isolation; needs `IGLOO_WORKER_DEV_MODE`). |
| `IGLOO_WORKER_DEV_MODE`        | `false`  | Allows the `process` runtime. Never set it for untrusted code.            |
| `IGLOO_WORKER_OCI_RUNTIME`     | `youki`  | `youki`, `crun` or `runc`, by name or path.                               |
| `IGLOO_WORKER_OVERLAY`         | `false`  | Builds sandbox file systems as overlay mounts. Linux, as root.            |
| `IGLOO_WORKER_LAYER_CACHE_MIB` | `20480`  | Layer cache kept beyond the layers sandboxes use.                         |
| `IGLOO_WORKER_LABELS`          | none     | Labels advertised on registration, as `key=value,other=value`.            |
| `RUST_LOG`                     | `info`   | Log filter.                                                               |

## CLI (`igloo`)

| Variable      | Flag      | Default                 | Meaning                               |
| ------------- | --------- | ----------------------- | ------------------------------------- |
| `IGLOO_API`   | `--api`   | `http://127.0.0.1:7000` | The server's REST API.                |
| `IGLOO_TOKEN` | `--token` | none                    | The bearer token (`IGLOO_DEV_TOKEN`). |

## Repository files

| File                   | Read from                          | Holds                                             | Schema                         |
| ---------------------- | ---------------------------------- | ------------------------------------------------- | ------------------------------ |
| `.igloo/pipeline.toml` | The commit being checked           | Image, warm command, sandbox, environment, checks | `schemas/pipeline.schema.json` |
| `.igloo/agents.toml`   | The default branch, never a change | Protected paths, default tool, coding tools       | `schemas/agents.schema.json`   |

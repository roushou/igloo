# Deploying Igloo on one host

`deploy/vps.sh` runs Igloo on a Linux host with systemd and apt (Ubuntu): Postgres, the server as
the `igloo` user and a root worker with crun and overlays. Everything listens on localhost only;
reach it through an SSH tunnel.

| Path                       | Holds                                                                  |
| -------------------------- | ---------------------------------------------------------------------- |
| `/etc/igloo/control.env`   | Server settings and secrets. Back it up: it holds `IGLOO_SECRETS_KEY`. |
| `/etc/igloo/worker.env`    | Worker settings.                                                       |
| `/var/lib/igloo`           | Server data: blobs, repository mirrors, image downloads.               |
| `/var/lib/igloo-worker`    | Worker data: sandboxes and the layer cache.                            |
| `deploy/systemd/*.service` | The `igloo-control` and `igloo-worker` units.                          |

Keep both data directories on a disk with tens of GB free, never on a tmpfs `/tmp`.

## Once, on the host

```sh
git clone https://github.com/roushou/igloo ~/igloo
~/igloo/deploy/vps.sh install
```

`install` adds the packages, the user, the database and `/etc/igloo/*.env` when they are missing,
installs the units, then builds and starts Igloo. Rerunning it keeps existing settings and secrets.

## From the workstation

With [mise](https://mise.jdx.dev) and the host reachable as `ssh vps` (or `IGLOO_VPS` set):

| Task              | Does                                                                  |
| ----------------- | --------------------------------------------------------------------- |
| `mise run deploy` | Pulls `main` on the host, rebuilds, restarts and waits until it is up |
| `mise run tunnel` | Forwards the API and MCP to `http://127.0.0.1:7000`                   |
| `mise run token`  | Prints the API token for `IGLOO_TOKEN`                                |
| `mise run logs`   | Follows the server and worker logs                                    |

Then register repositories and secrets as in `docs/igloo-on-igloo.md`.

#!/usr/bin/env bash
# Runs Igloo on one Linux host: Postgres, the server as the `igloo` user, and a root worker with
# crun and overlays, all under systemd and bound to localhost. Run from the repository checkout as
# a user with sudo.
#
#   deploy/vps.sh install   Sets up the host, then runs `update`. Safe to rerun: existing
#                           configuration and secrets in /etc/igloo are kept.
#   deploy/vps.sh update    Builds the binaries, installs them and restarts both services.
#   deploy/vps.sh token     Prints the API token (IGLOO_DEV_TOKEN).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
etc=/etc/igloo
bin=/usr/local/bin
binaries=(igloo-control igloo-worker igloo)

log() { printf '\033[1m==> %s\033[0m\n' "$*"; }
secret() { openssl rand -hex 32; }
setting() { sudo sed -n "s/^$1=//p" "$etc/control.env"; }

install_packages() {
  log "Packages"
  local missing=()
  for package in build-essential git curl openssl postgresql crun; do
    dpkg -s "$package" >/dev/null 2>&1 || missing+=("$package")
  done
  if ((${#missing[@]})); then
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${missing[@]}"
  fi
  if ! command -v cargo >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain none
    # shellcheck source=/dev/null
    . "$HOME/.cargo/env"
  fi
}

create_user() {
  log "User and directories"
  id igloo >/dev/null 2>&1 \
    || sudo useradd --system --create-home --home-dir /var/lib/igloo --shell /usr/sbin/nologin igloo
  sudo install -d -m 755 "$etc"
  sudo install -d -m 700 /var/lib/igloo-worker
}

configure() {
  log "Configuration"
  if ! sudo test -f "$etc/control.env"; then
    local password port
    password=$(openssl rand -hex 24)
    # The apt cluster takes the next free port when 5432 is in use.
    port=$(sudo -u postgres psql -tAc "SHOW port")
    if sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname = 'igloo'" | grep -q 1; then
      sudo -u postgres psql -qc "ALTER ROLE igloo PASSWORD '$password'"
    else
      sudo -u postgres psql -qc "CREATE ROLE igloo LOGIN PASSWORD '$password'"
    fi
    sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname = 'igloo'" | grep -q 1 \
      || sudo -u postgres createdb -O igloo igloo
    sudo install -m 640 -o root -g igloo /dev/null "$etc/control.env"
    sudo tee "$etc/control.env" >/dev/null <<EOF
IGLOO_DATABASE_URL=postgres://igloo:$password@127.0.0.1:$port/igloo
IGLOO_DEV_TOKEN=$(secret)
IGLOO_JOIN_TOKEN=$(secret)
IGLOO_BLOB_KEY=$(secret)
IGLOO_SECRETS_KEY=$(secret)
IGLOO_LISTEN=127.0.0.1:7000
IGLOO_GATEWAY_LISTEN=127.0.0.1:7001
IGLOO_PUBLIC_URL=http://127.0.0.1:7000
IGLOO_DATA_DIR=/var/lib/igloo
EOF
    echo "Created $etc/control.env. Back it up: losing IGLOO_SECRETS_KEY loses every stored secret."
  fi
  if ! sudo test -f "$etc/worker.env"; then
    sudo install -m 600 /dev/null "$etc/worker.env"
    sudo tee "$etc/worker.env" >/dev/null <<EOF
IGLOO_WORKER_SERVER=http://127.0.0.1:7001
IGLOO_WORKER_JOIN_TOKEN=$(setting IGLOO_JOIN_TOKEN)
IGLOO_WORKER_DATA_DIR=/var/lib/igloo-worker
IGLOO_WORKER_RUNTIME=oci
IGLOO_WORKER_OCI_RUNTIME=crun
IGLOO_WORKER_OVERLAY=true
EOF
  fi
}

install_units() {
  log "systemd units"
  sudo install -m 644 "$repo"/deploy/systemd/*.service /etc/systemd/system/
  sudo systemctl daemon-reload
  sudo systemctl enable igloo-control igloo-worker
}

update() {
  # shellcheck source=/dev/null
  [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
  log "Build"
  cargo build --release --locked --manifest-path "$repo/Cargo.toml" \
    -p igloo-control -p igloo-worker -p igloo-cli
  log "Install and restart"
  for binary in "${binaries[@]}"; do
    sudo install -m 755 "$repo/target/release/$binary" "$bin/$binary"
  done
  sudo systemctl restart igloo-control igloo-worker
  local token status
  token=$(setting IGLOO_DEV_TOKEN)
  for _ in $(seq 30); do
    status=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $token" \
      http://127.0.0.1:7000/v1/repos || true)
    if [[ $status == 200 ]] && systemctl is-active --quiet igloo-worker; then
      log "Igloo is up at http://127.0.0.1:7000"
      return
    fi
    sleep 1
  done
  echo "Igloo did not come up; see: sudo journalctl -u igloo-control -u igloo-worker -n 50" >&2
  exit 1
}

case "${1:-}" in
  install)
    install_packages
    create_user
    configure
    install_units
    update
    ;;
  update) update ;;
  token) setting IGLOO_DEV_TOKEN ;;
  *)
    echo "usage: $0 install|update|token" >&2
    exit 2
    ;;
esac

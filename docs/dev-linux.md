# A Linux worker for a server on macOS

Isolated sandboxes (`igloo run --isolated`) need a Linux worker running as root with an OCI runtime
and overlay file systems. On macOS, run the server natively and the worker in a Linux VM.

## Server (macOS)

The worker reaches the server through the VM's host address, so the server listens on all interfaces
and advertises the URL the worker downloads layers from:

```sh
docker compose -f dev/compose.yaml up -d --wait
IGLOO_DATABASE_URL=postgres://igloo:igloo@localhost:5432/igloo IGLOO_DEV_TOKEN=dev \
  IGLOO_JOIN_TOKEN=join IGLOO_BLOB_KEY=dev-blob-key-at-least-32-bytes-long \
  IGLOO_SECRETS_KEY=dev-secrets-key-at-least-32-bytes \
  IGLOO_LISTEN=0.0.0.0:7000 IGLOO_GATEWAY_LISTEN=0.0.0.0:7001 \
  IGLOO_PUBLIC_URL=http://$HOST:7000 \
  cargo run --bin igloo-control
```

`$HOST` is how the VM reaches macOS: `host.orb.internal` with OrbStack, `host.lima.internal` with
Lima.

## VM

OrbStack:

```sh
orb create ubuntu igloo
orb -m igloo
```

Lima:

```sh
limactl start --name igloo template://ubuntu
limactl shell igloo
```

Both mount your home directory at the same path, so the repository is visible in the VM. Inside it,
install Rust (`curl https://sh.rustup.rs -sSf | sh`), build tools and youki:

```sh
sudo apt-get install -y build-essential
curl -fsSL https://github.com/youki-dev/youki/releases/download/v0.7.0/youki-0.7.0-$(uname -m)-musl.tar.gz \
  | sudo tar -xz -C /usr/local/bin youki
```

## Worker (VM)

Build with a target directory inside the VM, then run as root:

```sh
cd /path/to/igloo
CARGO_TARGET_DIR=~/igloo-target cargo build --release -p igloo-worker
sudo IGLOO_WORKER_SERVER=http://$HOST:7001 IGLOO_WORKER_JOIN_TOKEN=join \
  IGLOO_WORKER_DATA_DIR=/var/lib/igloo-worker IGLOO_WORKER_RUNTIME=oci \
  IGLOO_WORKER_OCI_RUNTIME=youki IGLOO_WORKER_OVERLAY=true \
  ~/igloo-target/release/igloo-worker
```

`IGLOO_WORKER_OCI_RUNTIME` also accepts `crun` or `runc`, by name or path.

## Use it

From macOS, in any project:

```sh
export IGLOO_TOKEN=dev
igloo snapshot import rust:1.99-slim --platform linux/arm64   # linux/amd64 on Intel
igloo run --isolated --base <snapshot id> -- cargo test
```

Imported images contribute their file system, not their environment: set the image's variables
(for the rust image, `PATH` including `/usr/local/cargo/bin`, `CARGO_HOME` and `RUSTUP_HOME`) on the
sandbox.

## Tests

The isolation tests run on Linux as root:

```sh
sudo -E env "PATH=$PATH" IGLOO_TEST_OVERLAY=1 IGLOO_TEST_OCI=1 IGLOO_TEST_OCI_RUNTIME=youki \
  IGLOO_TEST_BUSYBOX=/bin/busybox cargo test -p igloo-worker
sudo -E env "PATH=$PATH" IGLOO_TEST_EXIT_GATE=1 cargo test -p igloo-control --test exit_gate
```

The busybox test needs a static busybox (`apt-get install busybox-static`); the exit gate needs
Docker for its Postgres, or `IGLOO_TEST_DATABASE_URL`.

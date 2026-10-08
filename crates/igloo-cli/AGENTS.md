# igloo-cli

The CLI, installed as `igloo`: a thin binary over `igloo-rs`. Workflows (such as running a
directory) belong in the SDK, where agents reuse them and tests exercise them. The only crate
allowed to print to stdout and stderr.

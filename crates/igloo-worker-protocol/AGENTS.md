# igloo-worker-protocol

The server <-> worker contract, generated from `proto/igloo/worker/v1/`, plus conversions to
and from `igloo-core`. Data only.

- Never reuse field numbers; mark removed fields `reserved`.
- `buf lint` and `buf breaking` must pass.

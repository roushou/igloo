# igloo-core

The pure domain: the vocabulary, re-exported at the crate root (`Id`, `Digest`, `Timestamp`, `Actor`, `Labels`, `Entity`,
`Resource`, `Event`, `Plan`) and platform primitives (sandbox, snapshot, worker, job, repo).

- No I/O, no async, no clock, no ID generation, no environment access.
- Every entity command and every `plan` branch has a scenario test.
- Serialized events have insta snapshots; changing one is a contract change.
- The `testing` module (feature `testing`) is the shared `Scenario` harness.

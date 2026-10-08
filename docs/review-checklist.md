# Review checklist

## Scope and contracts

- [ ] Only the task's scope changed; follow-ups are listed, not implemented.
- [ ] Contract surfaces changed only as the spec allows.
- [ ] No new crate, port, core trait or dependency without approval.
- [ ] Protected paths (`.github/CODEOWNERS`) changed only as the spec allows.

## Architecture

- [ ] `igloo-core` is pure: no I/O, async, clock or ID generation.
- [ ] Writes go through the command bus; inbound adapters only parse, dispatch and map.
- [ ] Wire types stay out of application signatures; conversions live in contract crates.
- [ ] One transaction per entity; cross-entity effects through events.
- [ ] Platform modules do not import product modules; trust settings live on `Repo`.

## Rust

- [ ] Behavior lives on its owning type; no free functions carrying behavior.
- [ ] Value objects are built only through validated conversions; `TryFrom` collects all
      errors.
- [ ] Errors are typed with stable codes and keep their sources; no `unwrap`, `expect`,
      `panic!`.
- [ ] No direct clock, ID generation or task spawning.
- [ ] No lock or `watch::Ref` across `.await`; channels are bounded.
- [ ] Names follow conventions; public items document their invariants; comments describe
      behavior, not history.

## Tests

- [ ] Every command and every `plan` branch has a scenario test, including idempotent and
      rejection paths.
- [ ] Every port adapter runs the conformance suite.
- [ ] Serialized contracts have insta snapshots; changes are intentional.
- [ ] No test was weakened, skipped or deleted.

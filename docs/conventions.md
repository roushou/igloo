# Igloo conventions

How code is written. Snippets show the target shape of APIs; small helpers are elided. When
code and this file disagree, follow this file and report the discrepancy.

## 1. Pragmatism

- Build now what is expensive to retrofit: identifiers, event schemas, entity shape, ports
  for side effects, contract versions, composition seams. Defer the rest until a task needs
  it, behind a seam that already exists.
- An abstraction needs a second use or a seam to justify it. No speculative generality, no
  framework where a function on a type does.
- Prefer standard tools (compiler, clippy, cargo-deny, cargo-shear) over custom checks.

## 2. Crates

- A crate earns its existence: a deployable binary, a published library, a contract shared by
  separate programs, or isolation of heavy or platform-specific dependencies. Everything else
  is a module.
- A crate owns its concern. It declares what it needs as traits it owns, exposes types it owns,
  and never reaches into another crate's internals.
- Boundaries come from APIs and the compiler, not from custom tooling.

## 3. Ownership

- Every concern has an owning type. Behavior lives on the type whose invariant it protects:
  validation on the value object, transitions on the entity, mapping in a conversion impl.
- No free functions, except `main`, tests, and module-private helpers with no domain meaning.
- Types own state; methods act on that state: `Thing::new(...)`, then `thing.method()`. A struct
  without meaningful state, or a trait used to group functions or to impose a call order, is a
  free function in disguise.
- Traits exist for real polymorphism: several implementations used generically or swapped
  (ports, adapters, entities in generic stores). Not as namespaces.
- Cross boundaries with `From`, `TryFrom`, `FromStr`, `Display`, `AsRef`. Never with ad hoc
  `to_x`/`from_x` functions when a trait exists for it.

## 4. Naming and modules

- Components are nouns, methods are verbs. Traits name a role (`Clock`, `Reconciler`) or, for
  markers, a property (`Prefixed`).
- Commands: verb-noun (`CreateSandbox`). Events: noun-past-participle (`SandboxCreated`).
  Errors: `<Thing>Error::<Problem>`.
- Command names: `"<entity>.<verb>"` (`"sandbox.stop"`). Error codes: `"<entity>.<problem>"`
  in snake case (`"sandbox.invalid_transition"`). Event kinds: `"igloo.<entity>.<event>"`.
- A module with submodules is `name/mod.rs`; a leaf module is `name.rs`. One concept per
  module.

## 5. Visibility and docs

- `pub(crate)` by default; each crate's public API is small and deliberate.
- Every public item has a doc comment stating its invariants. Comments describe what code
  does and guarantees, never how it came to be.
- `#[must_use]` where clippy requires it: methods returning `Self`.
- Fields of domain types are private; construction goes through validated constructors.
- `#[non_exhaustive]` on public enums and structs in published crates (`igloo-api`,
  `igloo-rs`). Internal crates stay exhaustive so the compiler finds every match.

## 6. Identifiers

`Id<T>` follows the TypeID specification: `<prefix>_<26 chars>`, the 128 bits of a UUIDv7 in
TypeID's base32 alphabet (first character `0`-`7`). Serde goes through `Display`/`FromStr`,
so wire, log and URL forms are identical.

```rust
pub trait Prefixed { const PREFIX: &'static str; }

pub struct Id<T: Prefixed> { raw: Uuid, _marker: PhantomData<fn() -> T> }

impl Prefixed for Sandbox { const PREFIX: &'static str = "sbx"; }
pub type SandboxId = Id<Sandbox>;
```

Implement `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`, `PartialOrd`, `Ord`, `Debug` by hand;
derives would require them on `T`. The domain never creates IDs; they come from the
`IdGenerator` port. Content addresses are `Digest`, not `Id<T>`.

## 7. Value objects

Parse, don't validate. The only way to build a value object is a fallible conversion; once
held, it is valid and never re-validated.

```rust
pub struct ResourceLimits { millicpus: u32, memory_mib: u32 }

impl ResourceLimits {
    pub fn new(millicpus: u32, memory_mib: u32) -> Result<Self, ResourceLimitsError> { /* bounds */ }
}

impl Default for ResourceLimits { /* secure, bounded defaults */ }
```

`FromStr` delegates to `TryFrom<&str>` when both make sense.

## 8. Conversions at boundaries

- Inbound wire -> domain: `TryFrom`, collecting every field error into `ValidationErrors`.
- Outbound domain -> wire: `From`, infallible.
- These impls live in the contract crates. Wire types never appear in application signatures.

```rust
impl TryFrom<rest::CreateSandboxRequest> for SandboxSpec {
    type Error = ValidationErrors;
    fn try_from(req: rest::CreateSandboxRequest) -> Result<Self, Self::Error> {
        let (snapshot, limits, labels) = Validator::new()
            .field("snapshot", req.snapshot.parse::<SnapshotId>())
            .field("limits", ResourceLimits::try_from(req.limits))
            .field("labels", Labels::try_from(req.labels))
            .finish()?;
        Ok(SandboxSpec::builder().snapshot(snapshot).limits(limits).labels(labels).build())
    }
}
```

## 9. Entities, resources, events

An entity is built with `Thing::new` and changed with methods. Each method either rejects the
change without recording anything, or records events that are applied to the state at once.

```rust
let mut sandbox = Sandbox::new(id, spec, now)?;   // records Created
sandbox.schedule(worker)?;                         // records Scheduled
sandbox.stop();                                    // records StopRequested
let events = sandbox.take_events();                // committed with the state
let sandbox = Sandbox::replay(history);            // rebuilt from the store
```

```rust
pub trait Entity: Prefixed + Sized {
    type Event: Event;

    fn id(&self) -> Id<Self>;
    /// State from its creation event; `None` means a corrupt history.
    fn from_created(event: &Self::Event) -> Option<Self>;
    /// Folds one event into the state. Infallible: events are facts.
    fn apply(&mut self, event: &Self::Event);
    /// The events recorded since creation or replay.
    fn take_events(&mut self) -> Vec<Self::Event>;
    /// Provided: `from_created` then `apply` for the rest.
    fn replay(events: impl IntoIterator<Item = Self::Event>) -> Option<Self>;
}

pub trait Resource: Entity {
    type Spec;
    type Status;
    type Action;

    fn spec(&self) -> &Self::Spec;
    fn status(&self) -> &Self::Status;
    fn labels(&self) -> &Labels;
    fn generation(&self) -> Generation;
    /// Pure. Compares spec with status at `now` and says what to do next.
    fn plan(&self, now: Timestamp) -> Plan<Self::Action>;
}

pub enum Plan<A> { Converged, Act(Vec<A>), Recheck { after: SignedDuration } }

pub trait Event: Serialize + DeserializeOwned + Send + 'static {
    const SCHEMA_VERSION: u16;
    /// Stable forever, e.g. "igloo.sandbox.stop_requested".
    fn kind(&self) -> &'static str;
}
```

- Entity methods are pure: no I/O, clock or randomness. A method that needs time takes
  `now: Timestamp`; IDs are passed in. The actor is attached to events at commit.
- Every change goes through a private `record(event)` that applies and stores the event, so the
  state always equals the fold of its events.
- A change that is already satisfied records nothing and returns `Ok`, not an error.
- An impossible change returns a typed error with a stable code.
- Spec changes bump `generation`. Status reports carry `observed_generation`; a report for a
  generation newer than the spec is rejected.
- Clients declare desired state; there are no imperative "start" operations.
- One transaction touches one entity. Cross-entity effects go through events.

## 10. Commands and handlers

A command is a plain struct. Its handler loads the target entity, calls one method, and
commits; the generic `EntityHandler` covers that with two closures.

```rust
pub trait Command: Send + 'static {
    type Output: Send + 'static;
    const NAME: &'static str;
}

pub struct StopSandbox { pub sandbox: SandboxId }

impl Command for StopSandbox {
    type Output = ();
    const NAME: &'static str = "sandbox.stop";
}

platform.command(EntityHandler::infallible(
    store,
    clock,
    |command: &StopSandbox| command.sandbox,
    |sandbox: &mut Sandbox, _: &StopSandbox, _now| sandbox.stop(),
))?;
```

- `EntityHandler::new` takes a change returning the entity's typed error; a lost commit race
  is retried from a fresh load. `CreateHandler` generates the id and commits a new entity.
- `RequestContext { actor, correlation_id, causation_id, idempotency_key }`; commands sent by
  controllers and reactors use `RequestContext::caused_by(event, actor)`.
- Cross-cutting concerns are tower layers on the bus, never code inside handlers.

## 11. Controllers, reactors, extensions

```rust
pub trait Reconciler<R: Resource>: Send + Sync + 'static {
    /// Executes the plan's actions. Idempotent: it may run any number of times.
    fn reconcile(&self, resource: &R, actions: Vec<R::Action>)
        -> impl Future<Output = Result<(), AppError>> + Send;
}

#[async_trait]
pub trait Reactor: Send + Sync + 'static {
    /// Stable; the reactor's checkpoint is stored under it.
    fn name(&self) -> &'static str;
    /// Handles one event at least once, in log order. Must be idempotent.
    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError>;
}

pub trait Extension: Send + 'static {
    fn name(&self) -> &'static str;
    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError>;
}
```

`Controller<R>` owns queueing, resync, requeue and backoff; a reconciler only acts. Each module
registers itself through one `Extension` impl; `main.rs` installs them.

## 12. Ports and adapters

- One narrow, object-safe trait per kind of side effect, held as `Arc<dyn Port>`.
- Generic conveniences go on an extension trait with a blanket impl, keeping the port
  object-safe:

```rust
pub trait IdGenerator: Send + Sync { fn next_uuid(&self) -> Uuid; }
pub trait IdGeneratorExt: IdGenerator {
    fn next<T: Prefixed>(&self) -> Id<T> { Id::from_uuid(self.next_uuid()) }
}
impl<G: IdGenerator + ?Sized> IdGeneratorExt for G {}
```

- Every port has a memory adapter and a conformance suite (`EntityStoreConformance<S>` with
  `run_all`). Every adapter runs the same suite.
- Only composition code names concrete adapters.

## 13. Generics, `dyn` and async

- Generic over the entity inside the application (handlers, stores, controllers): type
  safety at zero cost.
- `dyn` at composition boundaries, where configuration chooses the implementation.
- Native `async fn` in traits where `dyn` is not needed; `#[async_trait]` where it is.
- Typestate only where misuse is costly, such as a job result that can only be built from a
  held `Lease`.

## 14. Errors

- One `thiserror` enum per layer, joined with `#[from]` so `?` composes.
- Every error implements `ErrorCode { fn code(&self) -> &'static str }`.
- `AppError`: `NotFound`, `Conflict`, `Forbidden`, `Validation`, `Domain`, `Infrastructure`.
  Transport maps it to an RFC 9457 problem via `From`.
- Infrastructure errors keep their `#[source]` and never leak details to clients.

## 15. Builders and configuration

- Types with more than three fields use `bon::Builder`; missing required fields are compile
  errors. Defaults are secure: deny-all network, bounded timeouts and limits.
- Builders serve trusted code; external input always goes through `TryFrom`.
- Configuration: one validated type per binary, read with `from_env()`, which delegates to
  `from_vars(pairs)` so tests pass variables without touching the process environment. It
  names every variable it reads, rejects unknown ones under its prefix and reports every
  problem at once, by variable name. Defaults are named constants. Fail at startup, never at
  first use.
- Configuration never holds per-repository trust settings (grants, budgets, policy).

## 16. Concurrency

- Every background task is spawned through `TaskSupervisor` (a `JoinSet` plus a
  `CancellationToken`). Shutdown is graceful and bounded.
- Channels are bounded. Desired state travels on `watch` channels.
- Never hold a lock or a `watch::Ref` across `.await`.

## 17. Testing

| Kind        | Where                                            | Tool                            |
| ----------- | ------------------------------------------------ | ------------------------------- |
| Scenario    | Every entity command and every `plan` branch     | `igloo_core::testing::Scenario` |
| Property    | Value objects, ID round trips, entity invariants | proptest                        |
| Snapshot    | Serialized events, API responses, problems       | insta                           |
| Conformance | Every port adapter                               | the port's conformance struct   |
| Integration | Postgres adapters                                | `sqlx::test` or testcontainers  |
| End to end  | Server + worker + CLI                            | in-process platform             |

```rust
#[test]
fn stopping_a_stopped_sandbox_is_a_no_op() {
    Scenario::<Sandbox>::given([created(), stop_requested(), status_recorded(Phase::Stopped)])
        .when(SandboxCommand::Stop)
        .then_no_events();
}
```

`Scenario` uses a fixed clock and a fixed actor. Test helpers live in the module's
`#[cfg(test)]` block.

## 18. Forbidden

| Forbidden                                         | Use instead                 |
| ------------------------------------------------- | --------------------------- |
| `SystemTime::now`, `jiff::Timestamp::now`         | `Clock`                     |
| `Uuid::new_v4`, `Uuid::now_v7`                    | `IdGenerator`               |
| `tokio::spawn`                                    | `TaskSupervisor::spawn`     |
| `unwrap`, `expect`, `panic!` outside tests        | `?` and typed errors        |
| `anyhow`/`eyre` outside a binary's `main.rs`      | `thiserror` enums           |
| Free functions carrying behavior                  | a method on the owning type |
| Stringly typed IDs, statuses, codes               | newtypes and enums          |
| Cargo features that change behavior across crates | separate types or crates    |
| `name.rs` next to `name/`                         | `name/mod.rs`               |
| Global trust settings (grants, budgets, policy)   | settings on `Repo`          |
| Comments narrating history or decisions           | an ADR                      |

## 19. Web console

`web/` is TypeScript, formatted and linted by Biome and run with Bun (ADR 0012). Rust rules do not
apply there; these do.

- Layout: `src/routes/` holds TanStack Router file routes and nothing else (a route loads data and
  renders a page component); `src/components/` holds components, `src/components/ui/` the shadcn/ui
  primitives; `src/lib/` holds helpers and query definitions; `src/api/` holds the API client and
  the generated types.
- `src/api/schema.gen.ts` is generated from `schemas/openapi.json` by `bun run gen:api`. Never edit
  it; commit it with the schema change that caused it.
- Server data is read through TanStack Query. Query keys and `queryOptions` are defined in
  `src/lib/queries.ts`; components do not spell keys. Components do not keep copies of server data
  in state.
- `fetch` is called only inside `src/api/client.ts`. Everything else uses `ApiClient`, which
  attaches the token and handles 401.
- The build must not touch the browser at import time: SPA prerendering imports every module on
  the server.
- Tests sit beside the code as `*.test.ts(x)`, render the real route tree, and stub the server at
  `fetch`.
- Look: every colour, font and size is a token in `src/styles.css`; components use the token
  classes (`bg-card`, `text-muted-foreground`, `text-expedition`), never a hex value, and the six
  type sizes of that file only. Orange (`expedition`) marks what waits on the user and nothing else.
  A state is shown by `StatusPill`, with the six states of `lib/status.ts`.
- Motion goes through `motion` and `lib/motion.ts`, which turns every transition off under
  `prefers-reduced-motion`. Floating surfaces (`Modal`, menus, toasts) are the only elevation.
- Keyboard: a shortcut is added to `SHORTCUTS` in `lib/shortcuts.tsx`, which the `?` dialog lists,
  and tested. A page offers its actions to ⌘K with `useRegisterCommands` and a memoised list; a
  command whose rule is not met stays listed with its reason in `unmet`.
- Server times the API does not report yet have one place each in `lib/timing.ts`; pages render
  from it, so a new field is wired there.
- Tests mount the router on the document (`renderApp`), as the browser does, and run as a narrow
  viewport that asks for reduced motion.

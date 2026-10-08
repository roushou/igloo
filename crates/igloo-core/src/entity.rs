use jiff::SignedDuration;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Generation, Id, Labels, Prefixed, Timestamp};

/// A domain event: an immutable fact stored in the event log.
///
/// Invariant: `kind` values and the serialized form of a given `SCHEMA_VERSION` never change.
pub trait Event: Serialize + DeserializeOwned + Send + 'static {
    /// Bumped on a breaking change to the serialized form; older versions are upcast on read.
    const SCHEMA_VERSION: u16;

    /// The CloudEvents `type`, such as `"igloo.sandbox.stop_requested"`.
    fn kind(&self) -> &'static str;
}

/// Something with an identity whose state is the fold of its events.
///
/// Entities are built with an inherent `new` and changed through inherent methods. Each method
/// either rejects the change without recording anything, or records events that are applied
/// to the state immediately. The store commits the state and [`Entity::take_events`] together.
pub trait Entity: Prefixed + Sized {
    /// The entity's name in codes and messages, such as `"sandbox"`.
    const NAME: &'static str;

    /// The facts this entity records.
    type Event: Event;

    /// The entity's identity.
    fn id(&self) -> Id<Self>;

    /// The state described by a creation event, with nothing recorded. `None` for any other
    /// event, which means the stored history is corrupt.
    fn from_created(event: &Self::Event) -> Option<Self>;

    /// Folds one event into the state. Infallible: events are facts.
    fn apply(&mut self, event: &Self::Event);

    /// Removes and returns the events recorded since creation or replay, oldest first.
    fn take_events(&mut self) -> Vec<Self::Event>;

    /// Rebuilds an entity from its full history. `None` if the history does not start with a
    /// creation event.
    fn replay(events: impl IntoIterator<Item = Self::Event>) -> Option<Self> {
        let mut events = events.into_iter();
        let mut entity = Self::from_created(&events.next()?)?;
        for event in events {
            entity.apply(&event);
        }
        Some(entity)
    }
}

/// An entity with a desired spec and an observed status, converged by a controller.
///
/// Invariant: every spec change bumps `generation`; status reports name the generation they
/// observed.
pub trait Resource: Entity {
    /// The desired state, declared by clients.
    type Spec;
    /// The observed state, reported by the system.
    type Status;
    /// What a reconciler may be asked to do.
    type Action;

    /// The desired state.
    fn spec(&self) -> &Self::Spec;

    /// The observed state.
    fn status(&self) -> &Self::Status;

    /// Metadata for grouping and filtering.
    fn labels(&self) -> &Labels;

    /// The current spec generation.
    fn generation(&self) -> Generation;

    /// Compares spec with status at `now` and says what to do next. Pure.
    fn plan(&self, now: Timestamp) -> Plan<Self::Action>;
}

/// The outcome of [`Resource::plan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan<A> {
    /// Nothing to do until something changes.
    Converged,
    /// Execute these actions, then plan again.
    Act(Vec<A>),
    /// Nothing to do now; plan again after this delay.
    Recheck {
        /// How long to wait before planning again.
        after: SignedDuration,
    },
}

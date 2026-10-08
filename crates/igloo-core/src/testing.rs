//! Given-when-then harness for entities and resources, on a fixed clock so every scenario is
//! deterministic.
//!
//! ```ignore
//! Scenario::<Sandbox>::given([created(), stop_requested()])
//!     .when(|sandbox, _| sandbox.stop())
//!     .then_no_events();
//! ```

#![allow(clippy::panic, reason = "a test harness reports failures by panicking")]

use std::convert::Infallible;
use std::fmt::Debug;

use jiff::SignedDuration;
use uuid::Uuid;

use crate::{Entity, ErrorCode, Id, Plan, Resource, Timestamp};

/// A scenario: an entity built from past events, ready for a change or a plan.
pub struct Scenario<E: Entity> {
    now: Timestamp,
    entity: Option<E>,
}

/// The events a change recorded, or its error, awaiting an assertion.
#[must_use = "a decision must be asserted with then, then_no_events or then_error"]
pub struct Decision<E: Entity, Err> {
    scenario: Scenario<E>,
    result: Result<Vec<E::Event>, Err>,
}

/// The plan of a resource, awaiting an assertion.
#[must_use = "a plan must be asserted with then_actions, then_converged or then_recheck"]
pub struct Planned<A> {
    plan: Plan<A>,
}

impl<E: Entity> Scenario<E> {
    /// The id to give entities created in a scenario.
    pub const ID: Id<E> = Id::from_uuid(Uuid::from_u128(1));

    /// The instant every scenario starts at: 2026-01-01T00:00:00Z.
    pub const NOW: Timestamp = Timestamp::new(jiff::Timestamp::constant(1_767_225_600, 0));

    /// An entity rebuilt from `events`, the first being its creation event.
    ///
    /// # Panics
    ///
    /// If `events` does not start with a creation event.
    pub fn given(events: impl IntoIterator<Item = E::Event>) -> Self {
        let Some(entity) = E::replay(events) else {
            panic!("given() needs a history starting with a creation event");
        };
        Self {
            now: Self::NOW,
            entity: Some(entity),
        }
    }

    /// Creates an entity with an infallible constructor.
    pub fn create(construct: impl FnOnce(Timestamp) -> E) -> Decision<E, Infallible> {
        Self::try_create(|now| Ok(construct(now)))
    }

    /// Creates an entity with a fallible constructor.
    pub fn try_create<Err>(
        construct: impl FnOnce(Timestamp) -> Result<E, Err>,
    ) -> Decision<E, Err> {
        let now = Self::NOW;
        match construct(now) {
            Ok(mut entity) => Decision {
                result: Ok(entity.take_events()),
                scenario: Self {
                    now,
                    entity: Some(entity),
                },
            },
            Err(error) => Decision {
                result: Err(error),
                scenario: Self { now, entity: None },
            },
        }
    }

    /// Moves the scenario clock to `now`.
    #[must_use]
    pub fn at(mut self, now: Timestamp) -> Self {
        self.now = now;
        self
    }

    /// Moves the scenario clock forward by `duration`.
    #[must_use]
    pub fn after(self, duration: SignedDuration) -> Self {
        let now = self.now.saturating_add(duration);
        self.at(now)
    }

    /// Applies an infallible change.
    pub fn when(self, change: impl FnOnce(&mut E, Timestamp)) -> Decision<E, Infallible> {
        self.try_when(|entity, now| {
            change(entity, now);
            Ok(())
        })
    }

    /// Applies a fallible change; its success value is ignored.
    ///
    /// # Panics
    ///
    /// If the scenario has no entity, or if a rejected change recorded events.
    pub fn try_when<T, Err>(
        mut self,
        change: impl FnOnce(&mut E, Timestamp) -> Result<T, Err>,
    ) -> Decision<E, Err> {
        let Some(entity) = self.entity.as_mut() else {
            panic!("when() needs an existing entity");
        };
        let outcome = change(entity, self.now);
        let events = entity.take_events();
        let result = match outcome {
            Ok(_) => Ok(events),
            Err(error) => {
                assert!(events.is_empty(), "a rejected change must record nothing");
                Err(error)
            }
        };
        Decision {
            scenario: self,
            result,
        }
    }

    /// The current entity.
    ///
    /// # Panics
    ///
    /// If the scenario has no entity.
    #[must_use]
    pub fn state(&self) -> &E {
        let Some(entity) = &self.entity else {
            panic!("the scenario has no entity");
        };
        entity
    }
}

impl<R: Resource> Scenario<R> {
    /// Plans the resource at the scenario clock.
    pub fn plan(&self) -> Planned<R::Action> {
        Planned {
            plan: self.state().plan(self.now),
        }
    }
}

impl<E: Entity, Err: Debug> Decision<E, Err>
where
    E::Event: PartialEq + Debug,
{
    /// Asserts the change recorded exactly `expected`.
    ///
    /// # Panics
    ///
    /// If the change failed or recorded other events.
    pub fn then(self, expected: impl IntoIterator<Item = E::Event>) -> Scenario<E> {
        let expected: Vec<E::Event> = expected.into_iter().collect();
        assert_eq!(
            self.result.as_ref().ok(),
            Some(&expected),
            "{:?}",
            self.result
        );
        self.scenario
    }

    /// Asserts the change succeeded without events: it was already satisfied.
    ///
    /// # Panics
    ///
    /// If the change failed or recorded events.
    pub fn then_no_events(self) -> Scenario<E> {
        self.then([])
    }
}

impl<E: Entity, Err: ErrorCode + Debug> Decision<E, Err>
where
    E::Event: Debug,
{
    /// Asserts the change was rejected with the error `code`.
    ///
    /// # Panics
    ///
    /// If the change succeeded or failed with another code.
    pub fn then_error(self, code: &str) -> Scenario<E> {
        let actual = self.result.as_ref().err().map(ErrorCode::code);
        assert_eq!(actual, Some(code), "{:?}", self.result);
        self.scenario
    }
}

impl<A: PartialEq + Debug> Planned<A> {
    /// Asserts the plan is to execute exactly `expected`.
    ///
    /// # Panics
    ///
    /// If the plan differs.
    pub fn then_actions(self, expected: impl IntoIterator<Item = A>) {
        assert_eq!(self.plan, Plan::Act(expected.into_iter().collect()));
    }

    /// Asserts there is nothing to do.
    ///
    /// # Panics
    ///
    /// If the plan differs.
    pub fn then_converged(self) {
        assert_eq!(self.plan, Plan::Converged);
    }

    /// Asserts the plan is to check again after `after`.
    ///
    /// # Panics
    ///
    /// If the plan differs.
    pub fn then_recheck(self, after: SignedDuration) {
        assert_eq!(self.plan, Plan::Recheck { after });
    }
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;
    use serde::{Deserialize, Serialize};

    use super::Scenario;
    use crate::{
        Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
    };

    /// A counter whose spec is a target value and whose status is the current value.
    #[derive(Debug)]
    struct Counter {
        id: Id<Counter>,
        target: u32,
        current: u32,
        generation: Generation,
        labels: Labels,
        updated_at: Timestamp,
        events: Vec<CounterEvent>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    enum CounterEvent {
        Created {
            id: Id<Counter>,
            target: u32,
            at: Timestamp,
        },
        Retargeted {
            target: u32,
        },
        Observed {
            current: u32,
            at: Timestamp,
        },
    }

    #[derive(Debug, PartialEq, Eq)]
    struct TooLarge;

    #[derive(Debug, PartialEq)]
    enum CounterAction {
        Increment,
    }

    impl ErrorCode for TooLarge {
        fn code(&self) -> &'static str {
            "counter.too_large"
        }
    }

    impl Counter {
        const MAX: u32 = 10;

        fn new(id: Id<Self>, target: u32, now: Timestamp) -> Result<Self, TooLarge> {
            if target > Self::MAX {
                return Err(TooLarge);
            }
            let created = CounterEvent::Created {
                id,
                target,
                at: now,
            };
            let mut counter = Self::initial(id, target, now);
            counter.events.push(created);
            Ok(counter)
        }

        fn initial(id: Id<Self>, target: u32, at: Timestamp) -> Self {
            Self {
                id,
                target,
                current: 0,
                generation: Generation::INITIAL,
                labels: Labels::default(),
                updated_at: at,
                events: Vec::new(),
            }
        }

        fn retarget(&mut self, target: u32) -> Result<(), TooLarge> {
            if target > Self::MAX {
                return Err(TooLarge);
            }
            if target != self.target {
                self.record(CounterEvent::Retargeted { target });
            }
            Ok(())
        }

        fn observe(&mut self, current: u32, now: Timestamp) {
            self.record(CounterEvent::Observed { current, at: now });
        }

        fn record(&mut self, event: CounterEvent) {
            self.apply(&event);
            self.events.push(event);
        }
    }

    impl Prefixed for Counter {
        const PREFIX: &'static str = "cnt";
    }

    impl Event for CounterEvent {
        const SCHEMA_VERSION: u16 = 1;

        fn kind(&self) -> &'static str {
            "igloo.counter.event"
        }
    }

    impl Entity for Counter {
        const NAME: &'static str = "counter";

        type Event = CounterEvent;

        fn id(&self) -> Id<Self> {
            self.id
        }

        fn from_created(event: &CounterEvent) -> Option<Self> {
            let CounterEvent::Created { id, target, at } = event else {
                return None;
            };
            Some(Self::initial(*id, *target, *at))
        }

        fn apply(&mut self, event: &CounterEvent) {
            match event {
                CounterEvent::Created { .. } => {}
                CounterEvent::Retargeted { target } => {
                    self.target = *target;
                    self.generation = self.generation.next();
                }
                CounterEvent::Observed { current, at } => {
                    self.current = *current;
                    self.updated_at = *at;
                }
            }
        }

        fn take_events(&mut self) -> Vec<CounterEvent> {
            std::mem::take(&mut self.events)
        }
    }

    impl Resource for Counter {
        type Spec = u32;
        type Status = u32;
        type Action = CounterAction;

        fn spec(&self) -> &u32 {
            &self.target
        }

        fn status(&self) -> &u32 {
            &self.current
        }

        fn labels(&self) -> &Labels {
            &self.labels
        }

        fn generation(&self) -> Generation {
            self.generation
        }

        /// Increments while below target, but waits a second after each observation.
        fn plan(&self, now: Timestamp) -> Plan<CounterAction> {
            let cooldown = SignedDuration::from_secs(1);
            let elapsed = now.duration_since(self.updated_at);
            if self.current >= self.target {
                Plan::Converged
            } else if elapsed < cooldown {
                Plan::Recheck {
                    after: cooldown - elapsed,
                }
            } else {
                Plan::Act(vec![CounterAction::Increment])
            }
        }
    }

    type S = Scenario<Counter>;

    fn created(target: u32) -> CounterEvent {
        CounterEvent::Created {
            id: S::ID,
            target,
            at: S::NOW,
        }
    }

    #[test]
    fn creation_records_the_creation_event() {
        S::try_create(|now| Counter::new(S::ID, 3, now)).then([created(3)]);
    }

    #[test]
    fn infallible_creation_is_supported() {
        S::create(|now| Counter::initial(S::ID, 3, now)).then_no_events();
    }

    #[test]
    fn creation_can_be_rejected() {
        S::try_create(|now| Counter::new(S::ID, 11, now)).then_error("counter.too_large");
    }

    #[test]
    fn changes_record_and_apply_events() {
        let scenario = S::given([created(3)])
            .try_when(|counter, _| counter.retarget(5))
            .then([CounterEvent::Retargeted { target: 5 }]);
        assert_eq!(scenario.state().target, 5);
        assert_eq!(scenario.state().generation, Generation::INITIAL.next());
    }

    #[test]
    fn satisfied_changes_record_nothing() {
        S::given([created(3)])
            .try_when(|counter, _| counter.retarget(3))
            .then_no_events();
    }

    #[test]
    fn rejected_changes_report_their_code() {
        S::given([created(3)])
            .try_when(|counter, _| counter.retarget(99))
            .then_error("counter.too_large");
    }

    #[test]
    fn plans_use_the_scenario_clock() {
        let scenario = S::given([created(2)]);
        scenario.plan().then_recheck(SignedDuration::from_secs(1));
        let scenario = scenario.after(SignedDuration::from_secs(1));
        scenario.plan().then_actions([CounterAction::Increment]);
        let observed_at = S::NOW.saturating_add(SignedDuration::from_secs(1));
        scenario
            .when(|counter, now| counter.observe(2, now))
            .then([CounterEvent::Observed {
                current: 2,
                at: observed_at,
            }])
            .plan()
            .then_converged();
    }

    #[test]
    #[should_panic(expected = "assertion")]
    fn mismatches_fail_with_expected_and_actual() {
        S::given([created(3)])
            .try_when(|counter, _| counter.retarget(5))
            .then([CounterEvent::Retargeted { target: 6 }]);
    }
}

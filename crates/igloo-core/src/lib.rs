//! The pure domain of Igloo: the vocabulary every primitive is built from (identifiers,
//! digests, time, actors, labels, validation, and the `Entity`, `Resource` and `Event` traits)
//! and the platform primitives built on it (`sandbox`, `worker`, `job`, `snapshot`, `seal`, `repo`, `change`, `workspace`).
//!
//! Performs no I/O, has no async runtime, never reads the clock, generates IDs or touches the
//! environment.
//!
//! Published as an implementation detail of `igloo-rs`; it carries no stability promise of
//! its own.

mod actor;
mod digest;
mod entity;
mod error;
mod id;
mod labels;
mod time;
mod validation;
mod version;

pub mod build;
pub mod change;
pub mod dotfiles;
pub mod job;
pub mod process;
pub mod repo;
pub mod sandbox;
pub mod seal;
pub mod snapshot;
pub mod terminal;
pub mod worker;
pub mod workspace;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use actor::{Actor, Agent, AgentId, SystemComponent, User, UserId};
pub use digest::{Digest, DigestError};
pub use entity::{Entity, Event, Plan, Resource};
pub use error::ErrorCode;
pub use id::{Id, IdError, Prefixed};
pub use labels::{LabelError, LabelKey, LabelValue, Labels};
pub use time::Timestamp;
pub use validation::{Append, ValidationErrors, Validator};
pub use version::{Generation, Version, ZeroGeneration};

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn json_forms_are_stable() {
        let user: UserId =
            Id::from_uuid(Uuid::from_u128(0x0189_0a5d_ac96_774b_bcce_b302_099a_8057));
        let agent: AgentId = Id::from_uuid(Uuid::from_u128(2));
        let forms = serde_json::json!({
            "id": user,
            "digest": Digest::from_blake3([0xab; 32]),
            "timestamp": Timestamp::from(jiff::Timestamp::constant(1_767_225_600, 0)),
            "actors": [
                Actor::Human { user },
                Actor::Agent { agent, principal: user },
                Actor::System { component: SystemComponent::Controller },
            ],
            "labels": Labels::from_pairs([("session", "42")]).expect("valid"),
        });
        insta::assert_json_snapshot!(forms);
    }
}

use serde::{Deserialize, Serialize};

use super::{Id, Prefixed};

/// Marker for human user identities (`usr_...`).
pub enum User {}

impl Prefixed for User {
    const PREFIX: &'static str = "usr";
}

/// Marker for agent identities (`agt_...`).
pub enum Agent {}

impl Prefixed for Agent {
    const PREFIX: &'static str = "agt";
}

/// Identifies a human user.
pub type UserId = Id<User>;

/// Identifies an agent.
pub type AgentId = Id<Agent>;

/// Who performs an action. Recorded with every event.
///
/// Invariant: an agent always acts for a human principal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    /// A human acting directly.
    Human {
        /// The acting user.
        user: UserId,
    },
    /// An agent acting on behalf of a human.
    Agent {
        /// The acting agent.
        agent: AgentId,
        /// The human the agent acts for.
        principal: UserId,
    },
    /// The platform acting on its own, such as a controller converging a resource.
    System {
        /// The acting component.
        component: SystemComponent,
    },
}

impl Actor {
    /// The human responsible for the action, if any.
    #[must_use]
    pub const fn principal(&self) -> Option<UserId> {
        match self {
            Self::Human { user } => Some(*user),
            Self::Agent { principal, .. } => Some(*principal),
            Self::System { .. } => None,
        }
    }
}

/// A part of the platform that acts on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemComponent {
    /// A resource controller.
    Controller,
    /// An event reactor.
    Reactor,
    /// The worker gateway, reporting on behalf of workers.
    Gateway,
}

use igloo_core::Actor;

/// Decides whether an actor may run a command.
pub trait PolicyEngine: Send + Sync {
    /// `Ok` when `request` is allowed.
    fn authorize(&self, request: &Authorization<'_>) -> Result<(), Denied>;
}

/// One authorization question.
#[derive(Debug, Clone, Copy)]
pub struct Authorization<'a> {
    /// Who asks.
    pub actor: &'a Actor,
    /// The command's name, such as `"sandbox.stop"`.
    pub command: &'static str,
}

/// A refused authorization, with a reason safe to show the caller.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct Denied {
    reason: String,
}

impl Denied {
    /// A refusal for `reason`.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

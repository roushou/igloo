use igloo_core::Timestamp;

/// The source of the current time. The only way application code reads the clock.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Timestamp;
}

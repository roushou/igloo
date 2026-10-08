use igloo_core::{Id, Prefixed};
use uuid::Uuid;

/// The source of new identifiers. The only way application code creates IDs.
///
/// Invariant: never returns the same UUID twice.
pub trait IdGenerator: Send + Sync {
    /// A new UUID; production implementations return UUIDv7 so IDs sort by creation time.
    fn next_uuid(&self) -> Uuid;
}

/// Typed convenience over [`IdGenerator`], kept off the port so it stays object safe.
pub trait IdGeneratorExt: IdGenerator {
    /// A new typed ID.
    fn next<T: Prefixed>(&self) -> Id<T> {
        Id::from_uuid(self.next_uuid())
    }
}

impl<G: IdGenerator + ?Sized> IdGeneratorExt for G {}

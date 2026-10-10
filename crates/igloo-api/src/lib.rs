//! The public API contract of Igloo: REST request and response types and RFC 9457 problems,
//! plus conversions to and from `igloo-core` types.
//!
//! Data only: no HTTP code and no business logic. Inbound conversions are `TryFrom` and report
//! every invalid field; outbound conversions are `From`.
//!
//! Published as an implementation detail of `igloo-rs`; it carries no stability promise of its
//! own.

pub mod change;
pub mod event;
pub mod job;
pub mod list;
pub mod me;
pub mod problem;
pub mod repo;
pub mod run;
pub mod sandbox;
pub mod seal;
pub mod snapshot;
pub mod storage;
pub mod task;
pub mod terminal;
pub mod worker;
pub mod workspace;

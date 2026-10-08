/// An error with a stable, machine-readable code such as `"sandbox.invalid_transition"`.
///
/// Invariant: a code never changes once released; clients and policies match on it.
pub trait ErrorCode {
    /// The stable code, `"<entity>.<problem>"` in snake case.
    fn code(&self) -> &str;
}

impl ErrorCode for std::convert::Infallible {
    fn code(&self) -> &'static str {
        match *self {}
    }
}

//! State module defines trait to loosely reason about state type

/// `StateSnapshot` is a trait that when state can be committed or new state can be created from it
pub trait StateSnapshot {
    /// Error of committing `StateSnapshot`
    type CommitError;

    /// Create new snapshot on top of existing one
    fn snapshot(&self) -> Self;

    /// 1. Persistence of current state and all previous.
    ///
    fn commit(&self) -> Result<(), Self::CommitError>;
}

//! Trait that represents life time of the state
//!

/// Storage manager persistence and allows to work on state
/// Temporal placeholder for ForkManager
pub trait StorageManager {
    /// Type that can be consumed by `[crate::state_machine::stf::StateTransitionFunction]` in native context
    type NativeStorage;
    /// Type that is produced by `[crate::state_machine::stf::StateTransitionFunction]`
    type NativeChangeSet;

    /// Get latest native state
    fn get_native_storage(&self) -> Self::NativeStorage;
}

//! Trait that represents life time of the state
//!

/// Storage manager manager persistence and allows to work on state
/// Temporal placeholder for ForkManager
pub trait StateManager {
    /// Type that can be consumed by `[crate::state_machine::stf::StateTransitionFunction]` in native context
    ///
    type NativeState;
    /// Type that is produced by `[crate::state_machine::stf::StateTransitionFunction]`
    type NativeChangeSet;

    /// Type that can be consumed by `[crate::state_machine::stf::StateTransitionFunction]` in ZK context
    type ZkState;

    /// Get latest native state
    fn get_native_state(&self) -> Self::NativeState;

    /// Get latest zk state
    fn get_zk_state(&self) -> Self::ZkState;
}

//! Trait that represents life time of the state
//!

use crate::da::DaSpec;

/// Storage manager, that supports tree-like hierarchy of snapshots
/// So different rollup state can be mapped to DA state 1 to 1, including chain forks.
pub trait HierarchicalStorageManager<Da: DaSpec> {
    /// Type that can be consumed by `[crate::state_machine::stf::StateTransitionFunction]` in native context.
    type NativeStorage;
    /// Type that is produced by `[crate::state_machine::stf::StateTransitionFunction]`.
    type NativeChangeSet;

    /// Creates a storage that can be used for execution of given DA block,
    /// meaning that at will have access to previous state in same fork.
    fn create_storage_for(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<Self::NativeStorage>;

    /// Creates a storage, that have data from execution of given DA block and all previous
    /// Similar to executing [`create_storage_for`] of the next block after `block_header`
    /// ChangeSet from this storage cannot be saved, as it does not have association with particular block
    fn create_storage_after(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<Self::NativeStorage>;

    /// Adds [`Self::NativeChangeSet`] to the storage.
    /// [`DaSpec::BlockHeader`] must be provided for efficient consistency checking.
    fn save_change_set(
        &mut self,
        block_header: &Da::BlockHeader,
        change_set: Self::NativeChangeSet,
    ) -> anyhow::Result<()>;

    /// Finalizes snapshot on given block header
    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()>;
}

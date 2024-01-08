//! Trait that represents life time of the state
//!

use crate::da::DaSpec;

/// Storage manager, that supports tree-like hierarchy of snapshots
/// So different rollup state can be mapped to DA state 1 to 1, including chain forks.
pub trait HierarchicalStorageManager<Da: DaSpec> {
    /// Type that can be consumed by `[crate::state_machine::stf::StateTransitionFunction]` in native context.
    type StfState;
    /// Type that is produced by `[crate::state_machine::stf::StateTransitionFunction]`.
    type StfChangeSet;

    /// TBD
    type LedgerState;

    /// TBD
    type LedgerChangeSet;

    /// Creates a storage that can be used for execution of given DA block,
    /// meaning that at will have access to previous state in same fork.
    fn create_state_for(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)>;

    /// Creates a storage, that have data from execution of given DA block and all previous
    /// Similar to executing [`create_storage_for`] of the next block after `block_header`
    /// ChangeSet from this storage cannot be saved, as it does not have association with particular block
    fn create_state_after(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)>;

    /// Adds [`Self::StfChangeSet`] to the storage.
    /// [`DaSpec::BlockHeader`] must be provided for efficient consistency checking.
    fn save_change_set(
        &mut self,
        block_header: &Da::BlockHeader,
        stf_change_set: Self::StfChangeSet,
        ledger_change_set: Self::LedgerChangeSet,
    ) -> anyhow::Result<()>;

    /// Finalizes snapshot on given block header
    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()>;
}

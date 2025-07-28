//! Trait that represents life time of the state
//!

use crate::da::DaSpec;

/// Storage manager, that supports tree-like hierarchy of states.
/// So different rollup state can be mapped to DA state 1 to 1, including chain forks.
/// State type represents a reference point to the current state and allows to build proper change set for given block.
pub trait HierarchicalStorageManager<Da: DaSpec>: Send + Sync {
    /// Type that can be consumed by `[crate::state_machine::stf::StateTransitionFunction]` in native context.
    type StfState;
    /// Type that is produced by `[crate::state_machine::stf::StateTransitionFunction]`.
    type StfChangeSet;

    /// Type that can be consumed by a ledger module. A module which is tracks ledger history.
    type LedgerState;
    /// Type which is produced by a ledger.
    type LedgerChangeSet;

    /// Creates a state that can be used for execution of given DA block,
    /// meaning that at will have access to previous state in same fork.
    fn create_state_for(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)>;

    /// Creates a state, that have data from execution of given DA block and all previous
    /// Similar to executing [`Self::create_state_for`] of the next block after `block_header`
    /// ChangeSet from this storage cannot be saved, as it does not have association with particular block.
    fn create_state_after(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)>;

    /// Adds [`Self::StfChangeSet`] to the tree of states.
    /// [`DaSpec::BlockHeader`] must be provided for efficient consistency checking.
    fn save_change_set(
        &mut self,
        block_header: &Da::BlockHeader,
        stf_change_set: Self::StfChangeSet,
        ledger_change_set: Self::LedgerChangeSet,
    ) -> anyhow::Result<()>;

    /// Finalizes state on given block header.
    /// Usually means that this state won't be altered anymore and can be persisted.
    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()>;
}

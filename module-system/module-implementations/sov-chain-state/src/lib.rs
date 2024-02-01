#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

/// Contains the call methods used by the module
mod call;
#[cfg(test)]
mod tests;

mod genesis;
pub use genesis::*;

/// Hook implementation for the module
pub mod hooks;

/// The query interface with the module
#[cfg(feature = "native")]
mod query;
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "native")]
pub use query::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::da::Time;
use sov_modules_api::prelude::*;
use sov_modules_api::{DaSpec, Error, KernelModuleInfo, ValidityConditionChecker, WorkingSet};
use sov_state::codec::BcsCodec;
use sov_state::storage::kernel_state::VersionReader;
use sov_state::storage::KernelWorkingSet;
use sov_state::Storage;

/// Type alias that contains the height of a given transition
pub type TransitionHeight = u64;

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
/// Structure that contains the information needed to represent a single state transition.
pub struct StateTransitionId<Da: DaSpec, StateRoot> {
    da_block_hash: Da::SlotHash,
    post_state_root: StateRoot,
    validity_condition: Da::ValidityCondition,
}

impl<Da: DaSpec, StateRoot> StateTransitionId<Da, StateRoot> {
    /// Creates a new state transition. Only available for testing as we only want to create
    /// new state transitions from existing [`TransitionInProgress`].
    pub fn new(
        da_block_hash: Da::SlotHash,
        post_state_root: StateRoot,
        validity_condition: Da::ValidityCondition,
    ) -> Self {
        Self {
            da_block_hash,
            post_state_root,
            validity_condition,
        }
    }
}

impl<Da: DaSpec, StateRoot: Serialize + DeserializeOwned + Eq> StateTransitionId<Da, StateRoot> {
    /// Compare the transition block hash and state root with the provided input couple. If
    /// the pairs are equal, return [`true`].
    pub fn compare_hashes(
        &self,
        da_block_hash: &Da::SlotHash,
        post_state_root: &StateRoot,
    ) -> bool {
        self.da_block_hash == *da_block_hash && self.post_state_root == *post_state_root
    }

    /// Returns the post state root of a state transition
    pub fn post_state_root(&self) -> &StateRoot {
        &self.post_state_root
    }

    /// Returns the da block hash of a state transition
    pub fn da_block_hash(&self) -> &Da::SlotHash {
        &self.da_block_hash
    }

    /// Returns the validity condition associated with the transition
    pub fn validity_condition(&self) -> &Da::ValidityCondition {
        &self.validity_condition
    }

    /// Checks the validity condition of a state transition
    pub fn validity_condition_check<Checker: ValidityConditionChecker<Da::ValidityCondition>>(
        &self,
        checker: &mut Checker,
    ) -> Result<(), <Checker as ValidityConditionChecker<Da::ValidityCondition>>::Error> {
        checker.check(&self.validity_condition)
    }
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
/// Represents a transition in progress for the rollup.
pub struct TransitionInProgress<Da: DaSpec> {
    da_block_hash: Da::SlotHash,
    validity_condition: Da::ValidityCondition,
}

impl<Da: DaSpec> TransitionInProgress<Da> {
    /// Creates a new transition in progress
    pub fn new(da_block_hash: Da::SlotHash, validity_condition: Da::ValidityCondition) -> Self {
        Self {
            da_block_hash,
            validity_condition,
        }
    }
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(Clone, KernelModuleInfo)]
pub struct ChainState<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> {
    /// Address of the module.
    #[address]
    address: C::Address,

    /// The current block height
    // We use a standard StateValue here instead of a `KernelStateValue` to avoid a chicken-and-egg problem.
    // You need to load the current visible_height in order to create a `KernelWorkingSet`, which is itself
    // required in order to read a `KernelStateValue`. This value is still protected by the fact that it exists
    // on a kernel module, which will not be accessible to the runtime.
    #[state]
    visible_height: sov_modules_api::StateValue<TransitionHeight>,

    /// The real slot height of the rollup.
    // This value is also required to create a `KernelWorkingSet`. See note on `visible_height` above.
    #[state]
    true_height: sov_modules_api::StateValue<TransitionHeight>,

    /// The current time, as reported by the DA layer
    #[state]
    time: sov_modules_api::VersionedStateValue<Time>,

    /// A record of all previous state transitions which are available to the VM.
    /// Currently, this includes *all* historical state transitions, but that may change in the future.
    /// This state map is delayed by one transition. In other words - the transition that happens in time i
    /// is stored during transition i+1. This is mainly due to the fact that this structure depends on the
    /// rollup's root hash which is only stored once the transition has completed.
    // TODO: This should be a `VersionedStateMap`, so that recent values are not visible to user-space
    #[state]
    historical_transitions: sov_modules_api::StateMap<
        TransitionHeight,
        StateTransitionId<Da, <C::Storage as Storage>::Root>,
        BcsCodec,
    >,

    /// The transition that is currently processed
    #[state]
    in_progress_transition: sov_modules_api::KernelStateValue<TransitionInProgress<Da>, BcsCodec>,

    /// The genesis root hash.
    /// Set after the first transaction of the rollup is executed, using the `begin_slot` hook.
    // TODO: This should be made read-only
    #[state]
    genesis_hash: sov_modules_api::StateValue<<C::Storage as Storage>::Root>,

    /// The height of genesis
    // TODO: This should be made read-only
    #[state]
    genesis_height: sov_modules_api::StateValue<TransitionHeight>,
}

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> ChainState<C, Da> {
    /// Returns transition height in the current slot
    pub fn true_slot_height(&self, working_set: &mut WorkingSet<C>) -> TransitionHeight {
        self.true_height.get(working_set).unwrap_or_default()
    }

    /// Returns transition height in the current slot
    pub fn visible_slot_height(&self, working_set: &mut WorkingSet<C>) -> TransitionHeight {
        self.visible_height.get(working_set).unwrap_or_default()
    }

    /// Returns the current time, as reported by the DA layer
    pub fn get_time(&self, working_set: &mut impl VersionReader) -> Time {
        self.time
            .get_current(working_set)
            .expect("Time must be set at initialization")
    }

    /// Return the genesis hash of the module.
    pub fn get_genesis_hash(
        &self,
        working_set: &mut WorkingSet<C>,
    ) -> Option<<C::Storage as Storage>::Root> {
        self.genesis_hash.get(working_set)
    }

    /// Returns the genesis height of the module.
    pub fn get_genesis_height(&self, working_set: &mut WorkingSet<C>) -> Option<TransitionHeight> {
        self.genesis_height.get(working_set)
    }

    /// Returns the transition in progress of the module.
    pub fn get_in_progress_transition(
        &self,
        working_set: &mut KernelWorkingSet<C>,
    ) -> Option<TransitionInProgress<Da>> {
        self.in_progress_transition.get(working_set)
    }

    /// Returns the completed transition associated with the provided `transition_num`.
    pub fn get_historical_transitions(
        &self,
        transition_num: TransitionHeight,
        working_set: &mut WorkingSet<C>,
    ) -> Option<StateTransitionId<Da, <C::Storage as Storage>::Root>> {
        self.historical_transitions
            .get(&transition_num, working_set)
    }
}

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> sov_modules_api::KernelModule
    for ChainState<C, Da>
{
    type Context = C;

    type Config = ChainStateConfig;

    fn genesis(&self, config: &Self::Config, working_set: &mut WorkingSet<C>) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }
}

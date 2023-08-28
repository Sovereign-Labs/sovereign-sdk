#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

/// Contains the call methods used by the module
pub mod call;

/// Genesis state configuration
pub mod genesis;

/// Hook implementation for the module
pub mod hooks;

#[cfg(test)]
pub mod tests;

/// The query interface with the module
pub mod query;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::ValidityConditionChecker;
use sov_state::WorkingSet;

/// Type alias that contains the height of a given transition
pub type TransitionHeight = u64;

#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, Eq)]
/// Structure that contains the information needed to represent a single state transition.
pub struct StateTransitionId<Da: DaSpec> {
    da_block_hash: Da::SlotHash,
    post_state_root: [u8; 32],
    validity_condition: Da::ValidityCondition,
}

// Manually implement partialeq for StateTransitionId because derive generates the wrong bounds
impl<Da: DaSpec> PartialEq for StateTransitionId<Da> {
    fn eq(&self, other: &Self) -> bool {
        self.da_block_hash == other.da_block_hash
            && self.post_state_root == other.post_state_root
            && self.validity_condition == other.validity_condition
    }
}

impl<Da: DaSpec> StateTransitionId<Da> {
    /// Creates a new state transition. Only available for testing as we only want to create
    /// new state transitions from existing [`TransitionInProgress`].
    pub fn new(
        da_block_hash: Da::SlotHash,
        post_state_root: [u8; 32],
        validity_condition: Da::ValidityCondition,
    ) -> Self {
        Self {
            da_block_hash,
            post_state_root,
            validity_condition,
        }
    }
}

impl<Da: DaSpec> StateTransitionId<Da> {
    /// Compare the transition block hash and state root with the provided input couple. If
    /// the pairs are equal, return [`true`].
    pub fn compare_hashes(&self, da_block_hash: &Da::SlotHash, post_state_root: &[u8; 32]) -> bool {
        self.da_block_hash == *da_block_hash && self.post_state_root == *post_state_root
    }

    /// Returns the post state root of a state transition
    pub fn post_state_root(&self) -> [u8; 32] {
        self.post_state_root
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

#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, Eq)]
/// Represents a transition in progress for the rollup.
pub struct TransitionInProgress<Da: DaSpec> {
    da_block_hash: Da::SlotHash,
    validity_condition: Da::ValidityCondition,
}

// Manually impl PartialEq because derive generates the wrong bounds
impl<Da: DaSpec> PartialEq for TransitionInProgress<Da> {
    fn eq(&self, other: &Self) -> bool {
        self.da_block_hash == other.da_block_hash
            && self.validity_condition == other.validity_condition
    }
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
#[derive(ModuleInfo)]
pub struct ChainState<Ctx: sov_modules_api::Context, Da: DaSpec> {
    /// Address of the module.
    #[address]
    pub address: Ctx::Address,

    /// The current block height
    #[state]
    pub slot_height: sov_state::StateValue<TransitionHeight>,

    /// A record of all previous state transitions which are available to the VM.
    /// Currently, this includes *all* historical state transitions, but that may change in the future.
    /// This state map is delayed by one transition. In other words - the transition that happens in time i
    /// is stored during transition i+1. This is mainly due to the fact that this structure depends on the
    /// rollup's root hash which is only stored once the transition has completed.
    #[state]
    pub historical_transitions: sov_state::StateMap<TransitionHeight, StateTransitionId<Da>>,

    /// The transition that is currently processed
    #[state]
    pub in_progress_transition: sov_state::StateValue<TransitionInProgress<Da>>,

    /// The genesis root hash.
    /// Set after the first transaction of the rollup is executed, using the `begin_slot` hook.
    #[state]
    pub genesis_hash: sov_state::StateValue<[u8; 32]>,

    /// The height of genesis
    #[state]
    pub genesis_height: sov_state::StateValue<TransitionHeight>,
}

/// Initial configuration of the chain state
pub struct ChainStateConfig {
    /// Initial slot height
    pub initial_slot_height: TransitionHeight,
}

impl<Ctx: sov_modules_api::Context, Da: DaSpec> sov_modules_api::Module for ChainState<Ctx, Da> {
    type Context = Ctx;

    type Config = ChainStateConfig;

    type CallMessage = sov_modules_api::NonInstantiable;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<Ctx::Storage>,
    ) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }
}

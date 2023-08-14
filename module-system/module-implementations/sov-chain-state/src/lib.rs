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
#[cfg(feature = "native")]
pub mod query;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker};
use sov_state::WorkingSet;

#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, Eq)]
/// Structure that contains the information needed to represent a single state transition.
pub struct StateTransitionId<Cond: ValidityCondition> {
    da_block_hash: [u8; 32],
    post_state_root: [u8; 32],
    validity_condition: Cond,
}

impl<Cond: ValidityCondition> StateTransitionId<Cond> {
    /// Creates a new state transition. Only available for testing as we only want to create
    /// new state transitions from existing [`TransitionInProgress`].
    pub fn new(
        da_block_hash: [u8; 32],
        post_state_root: [u8; 32],
        validity_condition: Cond,
    ) -> Self {
        Self {
            da_block_hash,
            post_state_root,
            validity_condition,
        }
    }
}

impl<Cond: ValidityCondition> StateTransitionId<Cond> {
    /// Compare the transition block hash and state root with the provided input couple. If
    /// the pairs are equal, return [`true`].
    pub fn compare_hashes(&self, da_block_hash: &[u8; 32], post_state_root: &[u8; 32]) -> bool {
        self.da_block_hash == *da_block_hash && self.post_state_root == *post_state_root
    }

    /// Returns the post state root of a state transition
    pub fn post_state_root(&self) -> [u8; 32] {
        self.post_state_root
    }

    /// Returns the da block hash of a state transition
    pub fn da_block_hash(&self) -> [u8; 32] {
        self.da_block_hash
    }

    /// Checks the validity condition of a state transition
    pub fn validity_condition_check<Checker: ValidityConditionChecker<Cond>>(
        &self,
        checker: &mut Checker,
    ) -> Result<(), <Checker as ValidityConditionChecker<Cond>>::Error> {
        checker.check(&self.validity_condition)
    }
}

#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, Eq)]
/// Represents a transition in progress for the rollup.
pub struct TransitionInProgress<Cond> {
    da_block_hash: [u8; 32],
    validity_condition: Cond,
}

impl<Cond> TransitionInProgress<Cond> {
    /// Creates a new transition in progress
    pub fn new(da_block_hash: [u8; 32], validity_condition: Cond) -> Self {
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
pub struct ChainState<Ctx: sov_modules_api::Context, Cond: ValidityCondition> {
    /// Address of the module.
    #[address]
    pub address: Ctx::Address,

    /// The current block height
    #[state]
    pub slot_height: sov_state::StateValue<u64>,

    /// A record of all previous state transitions which are available to the VM.
    /// Currently, this includes *all* historical state transitions, but that may change in the future.
    /// This state map is delayed by one transition. In other words - the transition that happens in time i
    /// is stored during transition i+1. This is mainly due to the fact that this structure depends on the
    /// rollup's root hash which is only stored once the transition has completed.
    #[state]
    pub historical_transitions: sov_state::StateMap<u64, StateTransitionId<Cond>>,

    /// The transition that is currently processed
    #[state]
    pub in_progress_transition: sov_state::StateValue<TransitionInProgress<Cond>>,

    /// The genesis root hash.
    /// Set after the first transaction of the rollup is executed, using the `begin_slot` hook.
    #[state]
    pub genesis_hash: sov_state::StateValue<[u8; 32]>,
}

/// Initial configuration of the chain state
pub struct ChainStateConfig {
    /// Initial slot height
    pub initial_slot_height: u64,
}

impl<Ctx: sov_modules_api::Context, Cond: ValidityCondition> sov_modules_api::Module
    for ChainState<Ctx, Cond>
{
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

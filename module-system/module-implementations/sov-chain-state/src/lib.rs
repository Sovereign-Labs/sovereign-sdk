#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

/// Contains the call methods used by the module
pub mod call;

/// Genesis state configuration
pub mod genesis;

/// Hook implementation for the module
pub mod hooks;

/// The query interface with the module
#[cfg(feature = "native")]
pub mod query;
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "native")]
pub use query::{ChainStateRpcImpl, ChainStateRpcServer};
use sov_modules_api::{Error, ModuleInfo, ValidityCondition, ValidityConditionChecker};
use sov_state::WorkingSet;

/// Type alias that contains the height of a given transition
pub type TransitionHeight = u64;

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

    /// Returns the validity condition associated with the transition
    pub fn validity_condition(&self) -> &Cond {
        &self.validity_condition
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
#[derive(Clone, ModuleInfo)]
pub struct ChainState<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> {
    /// Address of the module.
    #[address]
    address: C::Address,

    /// The current block height
    #[state]
    slot_height: sov_state::StateValue<TransitionHeight>,

    /// A record of all previous state transitions which are available to the VM.
    /// Currently, this includes *all* historical state transitions, but that may change in the future.
    /// This state map is delayed by one transition. In other words - the transition that happens in time i
    /// is stored during transition i+1. This is mainly due to the fact that this structure depends on the
    /// rollup's root hash which is only stored once the transition has completed.
    #[state]
    historical_transitions:
        sov_state::StateMap<TransitionHeight, StateTransitionId<Da::ValidityCondition>>,

    /// The transition that is currently processed
    #[state]
    in_progress_transition: sov_state::StateValue<TransitionInProgress<Da::ValidityCondition>>,

    /// The genesis root hash.
    /// Set after the first transaction of the rollup is executed, using the `begin_slot` hook.
    #[state]
    genesis_hash: sov_state::StateValue<[u8; 32]>,

    /// The height of genesis
    #[state]
    genesis_height: sov_state::StateValue<TransitionHeight>,
}

/// Initial configuration of the chain state
pub struct ChainStateConfig {
    /// Initial slot height
    pub initial_slot_height: TransitionHeight,
}

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> ChainState<C, Da> {
    /// Returns transition height in the current slot
    pub fn get_slot_height(&self, working_set: &mut WorkingSet<C::Storage>) -> TransitionHeight {
        self.slot_height
            .get(working_set)
            .expect("Slot height should be set at initialization")
    }

    /// Return the genesis hash of the module.
    pub fn get_genesis_hash(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<[u8; 32]> {
        self.genesis_hash.get(working_set)
    }

    /// Returns the genesis height of the module.
    pub fn get_genesis_height(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<TransitionHeight> {
        self.genesis_height.get(working_set)
    }

    /// Returns the transition in progress of the module.
    pub fn get_in_progress_transition(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<TransitionInProgress<Da::ValidityCondition>> {
        self.in_progress_transition.get(working_set)
    }

    /// Returns the completed transition associated with the provided `transition_num`.
    pub fn get_historical_transitions(
        &self,
        transition_num: TransitionHeight,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<StateTransitionId<Da::ValidityCondition>> {
        self.historical_transitions
            .get(&transition_num, working_set)
    }
}

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> sov_modules_api::Module
    for ChainState<C, Da>
{
    type Context = C;

    type Config = ChainStateConfig;

    type CallMessage = sov_modules_api::NonInstantiable;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }
}

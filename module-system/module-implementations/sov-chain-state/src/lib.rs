pub mod call;
pub mod genesis;
pub mod hooks;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
pub mod query;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_rollup_interface::mocks::MockValidityCond;
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker};
use sov_state::WorkingSet;

#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, Eq)]
pub struct StateTransitionId<Cond: ValidityCondition> {
    da_block_hash: [u8; 32],
    post_state_root: [u8; 32],
    validity_condition: Cond,
}

impl StateTransitionId<MockValidityCond> {
    pub fn new(
        da_block_hash: [u8; 32],
        post_state_root: [u8; 32],
        validity_condition: MockValidityCond,
    ) -> Self {
        Self {
            da_block_hash,
            post_state_root,
            validity_condition,
        }
    }
}

impl<Cond: ValidityCondition> StateTransitionId<Cond> {
    pub fn compare_tx_hashes(&self, da_block_hash: [u8; 32], post_state_root: [u8; 32]) -> bool {
        self.da_block_hash == da_block_hash && self.post_state_root == post_state_root
    }

    pub fn post_state_root(&self) -> [u8; 32] {
        self.post_state_root
    }

    pub fn da_block_hash(&self) -> [u8; 32] {
        self.da_block_hash
    }

    pub fn validity_condition_check<Checker: ValidityConditionChecker<Cond>>(
        &self,
        checker: &mut Checker,
    ) -> Result<(), <Checker as ValidityConditionChecker<Cond>>::Error> {
        checker.check(&self.validity_condition)
    }
}

#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, Eq)]
pub struct TransitionInProgress<Cond> {
    da_block_hash: [u8; 32],
    validity_condition: Cond,
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
    #[state]
    pub historical_transitions: sov_state::StateMap<u64, StateTransitionId<Cond>>,

    /// The transition that is currently processed
    #[state]
    pub in_progress_transition: sov_state::StateValue<TransitionInProgress<Cond>>,

    /// The initial state hash
    #[state]
    pub genesis_hash: sov_state::StateValue<[u8; 32]>,
}

impl<Ctx: sov_modules_api::Context, Cond: ValidityCondition> sov_modules_api::Module
    for ChainState<Ctx, Cond>
{
    type Context = Ctx;

    type Config = ();

    type CallMessage = ();

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<Ctx::Storage>,
    ) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        _msg: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<Ctx::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        // The call logic
        Ok(sov_modules_api::CallResponse::default())
    }
}

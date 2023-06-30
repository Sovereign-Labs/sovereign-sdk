pub mod call;
pub mod genesis;
pub mod hooks;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
pub mod query;

use borsh::{BorshDeserialize, BorshSerialize};
use genesis::StoredCodeCommitment;
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_rollup_interface::zk::traits::{ValidityCondition, Zkvm};
use sov_state::WorkingSet;

#[derive(BorshSerialize, BorshDeserialize)]
pub struct LastFoldedConditionAndIndex<Cond> {
    pub last_condition_folded: Cond,
    pub index: u64,
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(ModuleInfo)]
pub struct ValidityConditions<Ctx: sov_modules_api::Context, Vm: Zkvm, Cond: ValidityCondition> {
    /// Address of the module.
    #[address]
    pub address: Ctx::Address,

    /// The code commitment to be used for verifying proofs
    #[state]
    pub commitment_to_allowed_verifier_method: sov_state::StateValue<StoredCodeCommitment<Vm>>,

    /// The most recent validity condition that has been succesfully folded
    #[state]
    pub last_condition_folded: sov_state::StateValue<LastFoldedConditionAndIndex<Cond>>,

    /// All of the remaining validity conditions that need to be folded
    #[state]
    pub conditions_to_be_folded: sov_state::StateMap<u64, Cond>,
}

impl<Ctx: sov_modules_api::Context, Vm: Zkvm, Cond: ValidityCondition> sov_modules_api::Module
    for ValidityConditions<Ctx, Vm, Cond>
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

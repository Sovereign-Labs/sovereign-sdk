#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

pub use call::*;
pub use genesis::*;
/// The response type used by RPC queries.
#[cfg(feature = "native")]
pub use query::*;
use sov_modules_api::{Context, Error, ModuleInfo, WorkingSet, Zkvm};
use sov_state::codec::BcsCodec;

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo)]
pub struct ProverIncentives<C: Context, Vm: Zkvm> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// The address of the token used for bonding provers
    #[state]
    pub bonding_token_address: sov_modules_api::StateValue<C::Address>,

    /// The code commitment to be used for verifying proofs
    #[state]
    pub commitment_of_allowed_verifier_method:
        sov_modules_api::StateValue<Vm::CodeCommitment, BcsCodec>,

    /// The set of registered provers and their bonded amount.
    #[state]
    pub bonded_provers: sov_modules_api::StateMap<C::Address, u64>,

    /// The minimum bond for a prover to be eligible for onchain verification
    #[state]
    pub minimum_bond: sov_modules_api::StateValue<u64>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<C>,
}

impl<C: Context, Vm: Zkvm> sov_modules_api::Module for ProverIncentives<C, Vm> {
    type Context = C;

    type Config = ProverIncentivesConfig<C, Vm>;

    type CallMessage = call::CallMessage;

    type Event = ();

    fn genesis(&self, config: &Self::Config, working_set: &mut WorkingSet<C>) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::BondProver(bond_amount) => {
                self.bond_prover(bond_amount, context, working_set)
            }
            call::CallMessage::UnbondProver => self.unbond_prover(context, working_set),
            call::CallMessage::VerifyProof(proof) => {
                self.process_proof(&proof, context, working_set)
            }
        }
        .map_err(|e| e.into())
    }
}

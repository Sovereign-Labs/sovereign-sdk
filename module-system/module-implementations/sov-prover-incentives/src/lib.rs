#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

/// The call methods specified in this module
pub use call::CallMessage;
/// The response type used by RPC queries.
#[cfg(feature = "native")]
pub use query::*;
use serde::{Deserialize, Serialize};
use sov_modules_api::{Context, Error, ModuleInfo, WorkingSet, Zkvm};
use sov_state::codec::BcsCodec;

/// Configuration of the prover incentives module. Specifies the
/// address of the bonding token, the minimum bond, the commitment to
/// the allowed verifier method and a set of initial provers with their
/// bonding amount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProverIncentivesConfig<C: Context, Vm: Zkvm> {
    /// The address of the token to be used for bonding.
    bonding_token_address: C::Address,
    /// The minimum bond for a prover.
    minimum_bond: u64,
    /// A code commitment to be used for verifying proofs
    commitment_of_allowed_verifier_method: Vm::CodeCommitment,
    /// A list of initial provers and their bonded amount.
    initial_provers: Vec<(C::Address, u64)>,
}

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

mod call;
mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

use borsh::{BorshDeserialize, BorshSerialize};
pub use call::CallMessage;
#[cfg(feature = "native")]
pub use query::Response;
use sov_modules_api::{Context, Error};
use sov_modules_macros::ModuleInfo;
use sov_rollup_interface::zk::Zkvm;
use sov_state::WorkingSet;

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

/// A wrapper around a code commitment which implements borsh
#[derive(Clone, Debug)]
pub struct StoredCodeCommitment<Vm: Zkvm> {
    commitment: Vm::CodeCommitment,
}

impl<Vm: Zkvm> BorshSerialize for StoredCodeCommitment<Vm> {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        bincode::serialize_into(writer, &self.commitment)
            .expect("Serialization to vec is infallible");
        Ok(())
    }
}

impl<Vm: Zkvm> BorshDeserialize for StoredCodeCommitment<Vm> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let commitment: Vm::CodeCommitment = bincode::deserialize_from(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(Self { commitment })
    }
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[cfg_attr(feature = "native", derive(sov_modules_macros::ModuleCallJsonSchema))]
#[derive(ModuleInfo)]
pub struct ProverIncentives<C: Context, Vm: Zkvm> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// The address of the token used for bonding provers
    #[state]
    pub bonding_token_address: sov_state::StateValue<C::Address>,

    /// The code commitment to be used for verifying proofs
    #[state]
    pub commitment_of_allowed_verifier_method: sov_state::StateValue<StoredCodeCommitment<Vm>>,

    /// The set of registered provers and their bonded amount.
    #[state]
    pub bonded_provers: sov_state::StateMap<C::Address, u64>,

    /// The minimum bond for a prover to be eligible for onchain verification
    #[state]
    pub minimum_bond: sov_state::StateValue<u64>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<C>,
}

impl<C: Context, Vm: Zkvm> sov_modules_api::Module for ProverIncentives<C, Vm> {
    type Context = C;

    type Config = ProverIncentivesConfig<C, Vm>;

    type CallMessage = call::CallMessage;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
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

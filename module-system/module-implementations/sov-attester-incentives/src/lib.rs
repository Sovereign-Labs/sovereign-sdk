pub mod call;
pub mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
pub mod query;

use borsh::{BorshDeserialize, BorshSerialize};
use call::Role;
use sov_modules_api::{Context, Error};
use sov_modules_macros::ModuleInfo;
use sov_rollup_interface::zk::traits::Zkvm;
use sov_state::{Storage, WorkingSet};

pub struct AttesterIncentivesConfig<C: Context, Vm: Zkvm> {
    /// The address of the token to be used for bonding.
    pub bonding_token_address: C::Address,
    /// The minimum bond for an attester.
    pub minimum_attester_bond: u64,
    /// The minimum bond for a challenger.
    pub minimum_challenger_bond: u64,
    /// A code commitment to be used for verifying proofs
    pub commitment_to_allowed_challenge_method: Vm::CodeCommitment,
    /// A list of initial provers and their bonded amount.
    pub initial_attesters: Vec<(C::Address, u64)>,
}

/// A wrapper around a code commitment which implements borsh serialization
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

/// The information about an attester's unbonding
#[derive(BorshDeserialize, BorshSerialize, Clone, Debug)]
pub struct UnbondingInfo {
    /// The height at which an attester is allowed to withdraw their tokens
    pub unbonding_initiated_height: u64,
    /// The number of tokens that the attester may withdraw
    pub amount: u64,
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(ModuleInfo)]
pub struct AttesterIncentives<C: sov_modules_api::Context, Vm: Zkvm> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// The address of the token used for bonding provers
    #[state]
    pub bonding_token_address: sov_state::StateValue<C::Address>,

    /// The code commitment to be used for verifying proofs
    #[state]
    pub commitment_to_allowed_challenge_method: sov_state::StateValue<StoredCodeCommitment<Vm>>,

    /// The set of bonded attesters and their bonded amount.
    #[state]
    pub bonded_attesters: sov_state::StateMap<C::Address, u64>,

    /// The set of unbonding attesters and their bonded amount.
    #[state]
    pub unbonding_attesters: sov_state::StateMap<C::Address, UnbondingInfo>,

    /// The set of bonded challengers and their bonded amount.
    #[state]
    pub bonded_challengers: sov_state::StateMap<C::Address, u64>,

    /// The minimum bond for an attester to be eligble
    #[state]
    pub minimum_attester_bond: sov_state::StateValue<u64>,

    /// The minimum bond for an attester to be eligble
    #[state]
    pub minimum_challenger_bond: sov_state::StateValue<u64>,

    /// The height of the most recent block which light clients know to be finalized
    #[state]
    pub light_client_finalized_height: sov_state::StateValue<u64>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<C>,
}

impl<C, Vm: Zkvm, S, P> sov_modules_api::Module for AttesterIncentives<C, Vm>
where
    C: sov_modules_api::Context<Storage = S>,
    S: Storage<Proof = P>,
    P: BorshDeserialize + BorshSerialize,
{
    type Context = C;

    type Config = AttesterIncentivesConfig<C, Vm>;

    type CallMessage = call::CallMessage<C>;

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
            call::CallMessage::BondAttester(bond_amount) => {
                self.bond_user_helper(bond_amount, context.sender(), Role::Attester, working_set)
            }
            call::CallMessage::BeginAttesterUnbonding => {
                self.begin_unbonding_attester(context, working_set)
            }
            call::CallMessage::FinishAttesterUnbonding => todo!(),
            call::CallMessage::BondChallenger(bond_amount) => {
                self.bond_user_helper(bond_amount, context.sender(), Role::Challenger, working_set)
            }
            call::CallMessage::UnbondChallenger => self.unbond_challenger(context, working_set),
            call::CallMessage::ProcessAttestation(attestation) => {
                self.process_attestation(attestation, context, working_set)
            }
            call::CallMessage::ProcessChallenge(proof) => {
                self.process_challenge(&proof, context, working_set)
            }
        }
        .map_err(|e| e.into())
    }
}

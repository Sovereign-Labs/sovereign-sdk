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
use sov_rollup_interface::zk::{ValidityCondition, Zkvm};
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
pub struct AttesterIncentives<C: sov_modules_api::Context, Vm: Zkvm, Cond: ValidityCondition> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// The amount of time it takes to a light client to be confident
    /// that an attested state transition won't be challenged. Measured in
    /// number of blocks.
    #[state]
    pub rollup_finality_period: sov_state::StateValue<u64>,

    /// The address of the token used for bonding provers
    #[state]
    pub bonding_token_address: sov_state::StateValue<C::Address>,

    /// The code commitment to be used for verifying proofs
    #[state]
    pub commitment_to_allowed_challenge_method: sov_state::StateValue<StoredCodeCommitment<Vm>>,

    /// The set of bonded attesters and their bonded amount.
    /// We don't need an unbonding set anymore because the
    /// attesters can only unbond if their last attestation
    /// was posted more than 24 hours ago.
    #[state]
    pub bonded_attesters: sov_state::StateMap<C::Address, u64>,

    /// The last attested block for each attester. If an attester
    /// posted an attestation less than 24 hours ago, he can't unbond.
    /// This saves us from doing a two-phase unbonding and maintains the
    /// following invariant: "to check the validity of an attestation,
    /// we only need to check that the attester was bonded at the time"
    #[state]
    pub last_attested_block: sov_state::StateMap<C::Address, u64>,

    /// The current maximum attestation height
    #[state]
    pub maximum_attested_height: sov_state::StateValue<u64>,

    /// TODO: if an attester has an attestation with a valid initial root but invalid post root
    /// we slash them and keep the bond so that people can challenge them

    /// Challengers now challenge a transition and not a specific attestation
    /// Mapping from an initial root hash to the associated reward value.
    /// This mapping is populated when the attestations are processed by the rollup
    #[state]
    pub bad_transition_pool: sov_state::StateMap<[u8; 32], u64>,

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

    /// Reference to the chain state module, used to check the initial hashes of the state transition.
    #[module]
    pub(crate) chain_state: sov_chain_state::ChainState<C, Cond>,
}

impl<C, Vm, S, P, Cond> sov_modules_api::Module for AttesterIncentives<C, Vm, Cond>
where
    C: sov_modules_api::Context<Storage = S>,
    Vm: Zkvm,
    S: Storage<Proof = P>,
    P: BorshDeserialize + BorshSerialize,
    Cond: ValidityCondition,
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
            call::CallMessage::UnbondAttester => {
                self.unbond_user_helper(context, Role::Attester, working_set)
            }
            call::CallMessage::BondChallenger(bond_amount) => {
                self.bond_user_helper(bond_amount, context.sender(), Role::Challenger, working_set)
            }
            call::CallMessage::UnbondChallenger => {
                self.unbond_user_helper(context, Role::Challenger, working_set)
            }
            call::CallMessage::ProcessAttestation(attestation) => {
                self.process_attestation(attestation, context, working_set)
            }
            call::CallMessage::ProcessChallenge(proof, initial_hash) => {
                self.process_challenge(&proof, initial_hash, context, working_set)
            }
        }
        .map_err(|e| e.into())
    }
}

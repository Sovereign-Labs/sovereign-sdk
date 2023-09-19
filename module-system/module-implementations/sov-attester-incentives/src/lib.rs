#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

/// Call methods for the module
pub mod call;

/// Methods used to instantiate the module
pub mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use call::Role;
#[cfg(feature = "native")]
pub use query::*;
use sov_bank::Amount;
use sov_chain_state::TransitionHeight;
use sov_modules_api::{
    Context, DaSpec, Error, ModuleInfo, ValidityConditionChecker, WorkingSet, Zkvm,
};
use sov_state::codec::BcsCodec;

/// Configuration of the attester incentives module
pub struct AttesterIncentivesConfig<C, Vm, Da, Checker>
where
    C: Context,
    Vm: Zkvm,
    Da: DaSpec,
    Checker: ValidityConditionChecker<Da::ValidityCondition>,
{
    /// The address of the token to be used for bonding.
    pub bonding_token_address: C::Address,
    /// The address of the account holding the reward token supply
    pub reward_token_supply_address: C::Address,
    /// The minimum bond for an attester.
    pub minimum_attester_bond: Amount,
    /// The minimum bond for a challenger.
    pub minimum_challenger_bond: Amount,
    /// A code commitment to be used for verifying proofs
    pub commitment_to_allowed_challenge_method: Vm::CodeCommitment,
    /// A list of initial provers and their bonded amount.
    pub initial_attesters: Vec<(C::Address, Amount)>,
    /// The finality period of the rollup (constant) in the number of DA layer slots processed.
    pub rollup_finality_period: TransitionHeight,
    /// The current maximum attested height
    pub maximum_attested_height: TransitionHeight,
    /// The light client finalized height
    pub light_client_finalized_height: TransitionHeight,
    /// The validity condition checker used to check validity conditions
    pub validity_condition_checker: Checker,
    /// Phantom data that contains the validity condition
    phantom_data: PhantomData<Da::ValidityCondition>,
}

/// The information about an attender's unbonding
#[derive(BorshDeserialize, BorshSerialize, Clone, Debug)]
pub struct UnbondingInfo {
    /// The height at which an attester started unbonding
    pub unbonding_initiated_height: TransitionHeight,
    /// The number of tokens that the attester may withdraw
    pub amount: Amount,
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(ModuleInfo)]
pub struct AttesterIncentives<C, Vm, Da, Checker>
where
    C: Context,
    Vm: Zkvm,
    Da: DaSpec,
    Checker: ValidityConditionChecker<Da::ValidityCondition>,
{
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// The amount of time it takes to a light client to be confident
    /// that an attested state transition won't be challenged. Measured in
    /// number of slots.
    #[state]
    pub rollup_finality_period: sov_modules_api::StateValue<TransitionHeight>,

    /// The address of the token used for bonding provers
    #[state]
    pub bonding_token_address: sov_modules_api::StateValue<C::Address>,

    /// The address of the account holding the reward token supply
    /// TODO: maybe mint the token before transferring it? The mint method is private in bank
    /// so we need a reward address that contains the supply.
    #[state]
    pub reward_token_supply_address: sov_modules_api::StateValue<C::Address>,

    /// The code commitment to be used for verifying proofs
    #[state]
    pub commitment_to_allowed_challenge_method:
        sov_modules_api::StateValue<Vm::CodeCommitment, BcsCodec>,

    /// Constant validity condition checker for the module.
    #[state]
    pub validity_cond_checker: sov_modules_api::StateValue<Checker>,

    /// The set of bonded attesters and their bonded amount.
    #[state]
    pub bonded_attesters: sov_modules_api::StateMap<C::Address, Amount>,

    /// The set of unbonding attesters, and the unbonding information (ie the
    /// height of the chain where they started the unbonding and their associated bond).
    #[state]
    pub unbonding_attesters: sov_modules_api::StateMap<C::Address, UnbondingInfo>,

    /// The current maximum attestation height
    #[state]
    pub maximum_attested_height: sov_modules_api::StateValue<TransitionHeight>,

    /// Challengers now challenge a transition and not a specific attestation
    /// Mapping from a transition number to the associated reward value.
    /// This mapping is populated when the attestations are processed by the rollup
    #[state]
    pub bad_transition_pool: sov_modules_api::StateMap<TransitionHeight, Amount>,

    /// The set of bonded challengers and their bonded amount.
    #[state]
    pub bonded_challengers: sov_modules_api::StateMap<C::Address, Amount>,

    /// The minimum bond for an attester to be eligble
    #[state]
    pub minimum_attester_bond: sov_modules_api::StateValue<Amount>,

    /// The minimum bond for an attester to be eligble
    #[state]
    pub minimum_challenger_bond: sov_modules_api::StateValue<Amount>,

    /// The height of the most recent block which light clients know to be finalized
    #[state]
    pub light_client_finalized_height: sov_modules_api::StateValue<TransitionHeight>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<C>,

    /// Reference to the chain state module, used to check the initial hashes of the state transition.
    #[module]
    pub(crate) chain_state: sov_chain_state::ChainState<C, Da>,
}

impl<C, Vm, Da, Checker> sov_modules_api::Module for AttesterIncentives<C, Vm, Da, Checker>
where
    C: sov_modules_api::Context,
    Vm: Zkvm,
    Da: DaSpec,
    Checker: ValidityConditionChecker<Da::ValidityCondition>,
{
    type Context = C;

    type Config = AttesterIncentivesConfig<C, Vm, Da, Checker>;

    type CallMessage = call::CallMessage<C, Da>;

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
            call::CallMessage::BondAttester(bond_amount) => self
                .bond_user_helper(bond_amount, context.sender(), Role::Attester, working_set)
                .map_err(|err| err.into()),
            call::CallMessage::BeginUnbondingAttester => self
                .begin_unbond_attester(context, working_set)
                .map_err(|error| error.into()),

            call::CallMessage::EndUnbondingAttester => self
                .end_unbond_attester(context, working_set)
                .map_err(|error| error.into()),
            call::CallMessage::BondChallenger(bond_amount) => self
                .bond_user_helper(bond_amount, context.sender(), Role::Challenger, working_set)
                .map_err(|err| err.into()),
            call::CallMessage::UnbondChallenger => self.unbond_challenger(context, working_set),
            call::CallMessage::ProcessAttestation(attestation) => self
                .process_attestation(context, attestation, working_set)
                .map_err(|error| error.into()),

            call::CallMessage::ProcessChallenge(proof, transition) => self
                .process_challenge(context, &proof, &transition, working_set)
                .map_err(|error| error.into()),
        }
        .map_err(|e| e.into())
    }
}

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

/// Call methods for the module
mod call;
mod capabilities;
/// Methods used to instantiate the module
mod genesis;
mod helpers;
mod registration;
pub use call::*;
pub use genesis::*;

#[cfg(feature = "native")]
mod query;

mod event;
use borsh::{BorshDeserialize, BorshSerialize};
pub use capabilities::{ProcessAttestationErrors, ProcessChallengeErrors};
#[cfg(feature = "native")]
pub use query::*;
pub use registration::CustomError;
use sov_bank::{Amount, BurnRate};
pub use sov_modules_api::optimistic::Attestation;
use sov_modules_api::runtime::OperatingMode;
use sov_modules_api::{
    Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, StateMap,
    StateReader, StateValue, TxState,
};
use sov_rollup_interface::common::SlotNumber;
use sov_state::User;

pub use crate::event::Event;

/// The information about an attender's unbonding
#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, Eq)]
pub struct UnbondingInfo {
    /// The height at which an attester started unbonding
    pub unbonding_initiated_height: SlotNumber,
    /// The number of tokens that the attester may withdraw
    pub amount: Amount,
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[id]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct AttesterIncentives<S>
where
    S: Spec,
{
    /// Id of the module.
    #[id]
    pub id: ModuleId,

    /// The amount of time it takes to a light client to be confident
    /// that an attested state transition won't be challenged. Measured in
    /// number of slots.
    #[state]
    pub rollup_finality_period: StateValue<SlotNumber>,

    /// The set of bonded attesters and their bonded amount.
    #[rest_api(include)]
    #[state]
    pub bonded_attesters: StateMap<S::Address, Amount>,

    /// The set of unbonding attesters, and the unbonding information (ie the
    /// height of the chain where they started the unbonding and their associated bond).
    #[state]
    pub unbonding_attesters: StateMap<S::Address, UnbondingInfo>,

    /// The current maximum attestation height
    #[state]
    pub maximum_attested_height: StateValue<SlotNumber>,

    /// Challengers now challenge a transition and not a specific attestation
    /// Mapping from a transition number to the associated reward value.
    /// This mapping is populated when the attestations are processed by the rollup
    #[state]
    pub bad_transition_pool: StateMap<SlotNumber, Amount>,

    /// The set of bonded challengers and their bonded amount.
    #[rest_api(include)]
    #[state]
    pub bonded_challengers: StateMap<S::Address, Amount>,

    /// The minimum bond for an attester to be eligble
    /// This should always be above the maximum gas limit to avoid collusion.
    ///
    /// This bond is expressed in gas units. When attesters are submitting attestations, they should
    /// have bonded at least the token value of this `minimum_bond` at the current `base_fee_per_gas`.
    #[state]
    pub minimum_attester_bond: StateValue<S::Gas>,

    /// The minimum bond for an attester to be eligble
    /// This should always be above the maximum gas limit to avoid collusion.
    ///
    /// This bond is expressed in gas units. When challengers are submitting challenges, they should
    /// have bonded at least the token value of this `minimum_bond` at the current `base_fee_per_gas`.
    #[state]
    pub minimum_challenger_bond: StateValue<S::Gas>,

    /// The height of the most recent block which light clients know to be finalized
    #[state]
    pub light_client_finalized_height: StateValue<SlotNumber>,

    /// The reward burn rate for the attester incentives module
    #[state]
    pub reward_burn_rate: StateValue<BurnRate>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<S>,

    /// Reference to the chain state module, used to check the initial hashes of the state transition.
    #[module]
    pub(crate) chain_state: sov_chain_state::ChainState<S>,
}

impl<S> Module for AttesterIncentives<S>
where
    S: Spec,
{
    type Spec = S;

    type Config = AttesterIncentivesConfig<S>;

    type CallMessage = call::CallMessage;

    type Event = Event<S>;

    type Error = anyhow::Error;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        // The initialization logic
        self.init_module(config, state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        if !self.should_reward_fees(state)? {
            return Err(anyhow::anyhow!(
                "Attester incentives call message received when operating in zk mode"
            ));
        }
        match msg {
            call::CallMessage::RegisterAttester(bond_amount) => {
                Ok(self.register_attester(bond_amount, context.sender(), state)?)
            }

            call::CallMessage::DepositAttester(amount) => {
                Ok(self.deposit_attester(amount, context.sender(), state)?)
            }

            call::CallMessage::BeginExitAttester => Ok(self.begin_exit_attester(context, state)?),

            call::CallMessage::ExitAttester => Ok(self.exit_attester(context, state)?),

            call::CallMessage::RegisterChallenger(bond_amount) => {
                Ok(self.register_challenger(bond_amount, context.sender(), state)?)
            }

            call::CallMessage::ExitChallenger => self.exit_challenger(context, state),
        }
    }
}

impl<S: Spec> AttesterIncentives<S> {
    /// Returns a bool indicating if the [`AttesterIncentives`] module should be paid fees.
    ///
    /// # Errors
    /// Returns an error if the operating mode cannot be read from `state`. With a gas-metered
    /// accessor (such as the one a transaction executes with), the read fails when the meter
    /// cannot pay for it, so callers must propagate this error rather than unwrap it.
    pub fn should_reward_fees<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<bool, Accessor::Error> {
        Ok(self.chain_state.operating_mode(state)? == OperatingMode::Optimistic)
    }
}

#[cfg(test)]
mod tests {
    use sov_modules_api::capabilities::mocks::MockKernel;
    use sov_modules_api::transaction::{PriorityFeeBips, TxDetails};
    use sov_modules_api::{
        Amount, AuthenticatedTransactionData, BasicGasMeter, Context, DaSpec, Gas, GasArray,
        Module, SequencerType, Spec, StateCheckpoint, StateProvider, WorkingSet,
    };
    use sov_rollup_interface::stf::ExecutionContext;
    use sov_state::nomt::zk_storage::NomtVerifierStorage;
    use sov_test_utils::ZkTestSpec;

    use super::*;

    type S = ZkTestSpec;

    /// The operating-mode check at the top of `call` is a gas-metered state read. If the
    /// transaction's meter cannot pay for it, the call must be rejected with an error.
    #[test]
    fn call_returns_error_when_meter_cannot_pay_for_operating_mode_read() {
        let checkpoint =
            StateCheckpoint::<S>::new(NomtVerifierStorage::new(), &MockKernel::<S>::default());
        let tx = AuthenticatedTransactionData::<S>(TxDetails {
            max_priority_fee_bips: PriorityFeeBips::ZERO,
            max_fee: Amount::ZERO,
            gas_limit: None,
            chain_hash_fragment: 0,
        });
        let exhausted_meter = BasicGasMeter::<S>::new_with_gas(
            <<S as Spec>::Gas as Gas>::zero(),
            <<S as Spec>::Gas as Gas>::Price::ZEROED,
        );
        let mut state =
            WorkingSet::create_working_set(checkpoint.to_tx_scratchpad(), &tx, exhausted_meter);

        let sender = <S as Spec>::Address::from([1; 28]);
        let context = Context::<S>::new(
            sender,
            Default::default(),
            sender,
            <<S as Spec>::Da as DaSpec>::Address::new([0; 32]),
            None,
            ExecutionContext::Node,
            SequencerType::NonPreferred,
        );

        let result = AttesterIncentives::<S>::default().call(
            call::CallMessage::BeginExitAttester,
            &context,
            &mut state,
        );

        assert!(
            result.is_err(),
            "A call that cannot pay for the operating mode read should return an error"
        );
    }
}

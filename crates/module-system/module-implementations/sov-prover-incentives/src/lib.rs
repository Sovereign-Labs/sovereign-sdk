#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod capabilities;
mod event;
mod genesis;
mod metrics;
mod registration;

pub use call::*;
pub use genesis::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_bank::Amount;
use sov_modules_api::runtime::OperatingMode;
use sov_modules_api::{
    AggregatedProofPublicData, CodeCommitmentFor, Context, DaSpec, Gas, GenesisState, GetGasPrice,
    ModuleId, ModuleInfo, ModuleRestApi, Spec, StateMap, StateReader, StateValue, TxState,
};
use sov_rollup_interface::common::SlotNumber;
use sov_state::codec::BcsCodec;
use sov_state::Storage;
use sov_state::User;

pub use crate::event::Event;

/// A snapshot of the rollup values that an admin upgrade replaces. Stored once per
/// upgrade, keyed by the proof's `final_slot_number`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "CodeCommitmentFor<S::OuterZkvm>: Serialize, CodeCommitmentFor<S::InnerZkvm>: Serialize, <S::Storage as Storage>::Root: Serialize",
    deserialize = "CodeCommitmentFor<S::OuterZkvm>: DeserializeOwned, CodeCommitmentFor<S::InnerZkvm>: DeserializeOwned, <S::Storage as Storage>::Root: DeserializeOwned",
))]
pub struct AdminUpgrade<S: Spec> {
    /// The outer code commitment claimed by the admin proof's public outputs.
    pub outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
    /// The inner code commitment claimed by the admin proof's public outputs.
    pub inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,
    /// The origin state root claimed by the admin proof's public outputs.
    pub origin_state_root: <S::Storage as Storage>::Root,
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[id]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct ProverIncentives<S: Spec> {
    /// Id of the module.
    #[id]
    pub id: ModuleId,

    /// The set of registered provers and their bonded amount.
    #[state]
    pub bonded_provers: StateMap<S::Address, Amount>,

    /// The minimum bond for a prover to be eligible for onchain verification
    ///
    /// This bond is expressed in gas units. When provers are submitting proofs, they should
    /// have bonded at least the token value of this `minimum_bond` at the current `base_fee_per_gas`.
    #[state]
    #[rest_api(include)]
    pub minimum_bond: StateValue<S::Gas>,

    /// The highest slot height for which the reward has been claimed. The next proofs should claim the next slot height.
    #[state]
    pub last_claimed_reward: StateValue<SlotNumber>,

    /// A penalty for provers who submit a proof for transitions that were already proven
    ///
    /// This quantity is expressed in gas units. When provers are penalized proofs, they will
    /// get penalized the token value of this `proving_penalty` at the current `base_fee_per_gas`.
    #[state]
    pub proving_penalty: StateValue<S::Gas>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<S>,

    /// Reference to the Chain state module. Used to check the proof inputs
    #[module]
    pub(crate) chain_state: sov_chain_state::ChainState<S>,

    /// Public data from the most recently accepted aggregated proof.
    #[state]
    #[allow(clippy::type_complexity)]
    pub latest_proof_succesfully_verified:
        StateValue<AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>>,

    /// The admin prover address.
    #[state]
    pub admin: StateValue<S::Address>,

    /// History of admin upgrades keyed by the final slot number of the admin proof
    /// that introduced the upgrade. Each entry carries the new outer + inner code
    /// commitments and the new origin state root that become canonical from that
    /// slot onward.
    #[state]
    pub admin_upgrades: StateMap<SlotNumber, AdminUpgrade<S>, BcsCodec>,

    /// The final slot number of the most recent admin upgrade.
    #[state]
    pub latest_admin_upgrade_slot: StateValue<SlotNumber>,
}

impl<S: Spec> sov_modules_api::Module for ProverIncentives<S> {
    type Spec = S;

    type Config = ProverIncentivesConfig<S>;

    type CallMessage = call::CallMessage;

    type Event = Event<S>;

    type Error = anyhow::Error;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<(), Self::Error> {
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
                "Prover incentives call message received when operating in optimistic mode"
            ));
        }
        match msg {
            call::CallMessage::Register(bond_amount) => {
                Ok(self.register(bond_amount, context.sender(), state)?)
            }
            call::CallMessage::Exit => Ok(self.exit(context.sender(), state)?),
            call::CallMessage::Deposit(bond_amount) => {
                Ok(self.deposit(bond_amount, context.sender(), state)?)
            }
        }
    }
}

impl<S: Spec> ProverIncentives<S> {
    /// Returns a bool indicating if the [`ProverIncentives`] module should be paid fees.
    ///
    /// # Errors
    /// Returns an error if the operating mode cannot be read from `state`. With a gas-metered
    /// accessor (such as the one a transaction executes with), the read fails when the meter
    /// cannot pay for it, so callers must propagate this error rather than unwrap it.
    pub fn should_reward_fees<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<bool, Accessor::Error> {
        Ok(self.chain_state.operating_mode(state)? == OperatingMode::Zk)
    }

    /// Returns the proving penalty as a [`u64`] value using the gas price contained in the state accessor.
    pub fn proving_penalty_value<State: TxState<S> + GetGasPrice<Spec = S>>(
        &self,
        state: &mut State,
    ) -> Result<Option<Amount>, <State as StateReader<User>>::Error> {
        self.proving_penalty
            .get(state)
            .map(|maybe_penalty| maybe_penalty.map(|penalty| penalty.value(state.gas_price())))
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
            chain_id: 0,
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

        let result =
            ProverIncentives::<S>::default().call(call::CallMessage::Exit, &context, &mut state);

        assert!(
            result.is_err(),
            "A call that cannot pay for the operating mode read should return an error"
        );
    }
}

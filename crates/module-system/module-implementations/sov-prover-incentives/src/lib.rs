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
        if !self.should_reward_fees(state) {
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
    pub fn should_reward_fees<Accessor: StateReader<User>>(&self, state: &mut Accessor) -> bool {
        self.chain_state
            .operating_mode(state)
            .expect("Operating mode retrieval should be infallible")
            == OperatingMode::Zk
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

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use sov_modules_api::capabilities::{BlockGasInfo, RollupHeight};
use sov_modules_api::prelude::UnwrapInfallible;
#[cfg(feature = "native")]
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::{
    AccessoryStateMap, AccessoryStateValue, ModuleRestApi, NotInstantiable,
    PrivilegedKernelAccessor, StateCheckpoint, StateMap, VersionReader,
};
/// Contains the call methods used by the module
mod call;
mod gas;
#[cfg(test)]
mod tests;
use sov_modules_api::{
    BootstrapWorkingSet, CodeCommitmentFor, GenesisState, KernelStateAccessor, KernelStateMap,
    ModuleId, ModuleInfo, Spec, StateAccessor, StateReader,
};

mod genesis;

pub use gas::{NonZeroRatio, NonZeroRatioConversionError};
pub use genesis::*;
use sov_modules_api::OperatingMode;

/// Capabilities implementation for the module
pub mod capabilities;

/// The query interface with the module
#[cfg(feature = "native")]
mod query;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::da::Time;
use sov_modules_api::{DaSpec, Gas, KernelStateValue, Module, StateValue, VersionedStateValue};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::codec::BcsCodec;
use sov_state::namespaces::Kernel;
use sov_state::{Storage, User};
use tracing::trace;

#[derive(Clone, Debug)]
/// A handy struct that groups the post state root of a slot with the information about the slot.
pub struct StateTransition<S: Spec> {
    post_state_root: <S::Storage as Storage>::Root,
    slot: SlotInformation<S>,
}

impl<S: Spec> StateTransition<S> {
    /// Creates a new state transition.
    pub fn new(post_state_root: <S::Storage as Storage>::Root, slot: SlotInformation<S>) -> Self {
        Self {
            slot,
            post_state_root,
        }
    }

    /// Returns the slot information of the state transition
    pub fn slot(&self) -> &SlotInformation<S> {
        &self.slot
    }

    /// Returns the post state root of the state transition
    pub fn post_state_root(&self) -> &<S::Storage as Storage>::Root {
        &self.post_state_root
    }
}

/// Represents a transition in progress for the rollup.
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(bound = "S: Spec")]
pub struct SlotInformation<S: Spec> {
    hash: <<S as Spec>::Da as DaSpec>::SlotHash,
    gas_info: BlockGasInfo<S::Gas>,
    prev_state_root: <S::Storage as Storage>::Root,
}

impl<S: Spec> SlotInformation<S> {
    /// Creates a new transition in progress
    pub fn new(
        slot_hash: <<S as Spec>::Da as DaSpec>::SlotHash,
        gas_info: BlockGasInfo<S::Gas>,
        prev_state_root: <S::Storage as Storage>::Root,
    ) -> Self {
        Self {
            hash: slot_hash,
            gas_info,
            prev_state_root,
        }
    }

    /// Returns the gas price of the transition.
    pub const fn gas_price(&self) -> &<S::Gas as Gas>::Price {
        self.gas_info.base_fee_per_gas()
    }

    /// Returns the total gas used of the transition.
    pub const fn gas_used(&self) -> &S::Gas {
        self.gas_info.gas_used()
    }

    /// Returns the gas limit of the transition.
    pub const fn gas_limit(&self) -> &S::Gas {
        self.gas_info.gas_limit()
    }

    /// Returns the hash of the DA block assocaited with this slot.
    pub const fn slot_hash(&self) -> &<<S as Spec>::Da as DaSpec>::SlotHash {
        &self.hash
    }

    /// Returns the pre state root for this slot.
    pub const fn prev_state_root(&self) -> &<S::Storage as Storage>::Root {
        &self.prev_state_root
    }

    /// Returns the gas info of the transition.
    pub fn gas_info(&self) -> &BlockGasInfo<S::Gas> {
        &self.gas_info
    }
}

/// The chain state module definition. Contains the current state of the da layer.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct ChainState<S: Spec> {
    /// The ID of the module.
    #[id]
    id: ModuleId,

    /// The height that should be loaded as the visible set at the start of the next block
    #[state]
    next_visible_slot_number: KernelStateValue<VisibleSlotNumber>,

    /// The rollup height of the current slot
    // This is a normal state value, since the current rollup height is always known
    #[state]
    current_heights: StateValue<(RollupHeight, VisibleSlotNumber)>,

    /// A mapping from rollup height to the visible slot number.
    #[state]
    slot_number_history: StateMap<RollupHeight, VisibleSlotNumber>,

    /// The slot number history duplicated in the accessory state.
    ///
    /// The duplication is required to enable the `finalize_hook` in materialize_slot, which needs to
    /// compute the visible hash using only accessory state.
    #[state]
    accessory_slot_number_history: AccessoryStateMap<RollupHeight, VisibleSlotNumber>,

    #[state]
    true_slot_number_history: AccessoryStateMap<RollupHeight, SlotNumber>,

    #[state]
    true_slot_number_to_rollup_height: AccessoryStateMap<SlotNumber, RollupHeight>,

    #[state]
    true_to_visible_slot_number_history: VersionedStateValue<VisibleSlotNumber>,

    /// The real rollup height of the rollup.
    /// This value is also required to create a [`sov_state::storage::KernelStateAccessor`]. See note on `visible_slot_number` above.
    #[state]
    true_slot_number: KernelStateValue<SlotNumber>,

    /// The current time, as reported by the DA layer
    #[state]
    time: VersionedStateValue<Time>,

    /// The mode that the rollup is operating in.
    #[state]
    operating_mode: StateValue<OperatingMode>,

    /// A record of all previous slots' information which are available to the VM.
    /// Currently, this includes *all* slots, but that may change in the future.
    #[state]
    slots: VersionedStateValue<SlotInformation<S>, BcsCodec>,

    /// A record of all the pre-state roots for each slot, stored in the accessory state.
    ///
    /// The duplication is required to enable the `finalize_hook` in materialize_slot, which needs to
    /// compute the visible hash using only accessory state.
    #[state]
    accessory_pre_state_roots:
        AccessoryStateMap<SlotNumber, <<S as Spec>::Storage as Storage>::Root>,

    /// The genesis state root of the rollup.
    #[state]
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    // This value is unreachable when the `native` feature is disabled
    genesis_root: AccessoryStateValue<<S::Storage as Storage>::Root>,

    /// A record of all previous rollup heights' gas information.
    #[state]
    gas_info: StateMap<RollupHeight, BlockGasInfo<S::Gas>>,

    /// The state root hashes from genesis to the current slot.
    /// ## Note
    /// There is a one slot-delay for the update of this state map because we cannot predict what will be the next
    /// most up to date state root inside the current slot. We have to wait for the next slot to start getting processed and return
    /// the pre-state root.
    #[state]
    past_user_state_roots: KernelStateMap<RollupHeight, [u8; 32]>,

    /// The state root hashes from genesis to the current slot, duplicated into accessory state.
    ///
    /// The duplication is required to enable the `finalize_hook` in materialize_slot, which needs to
    /// compute the visible hash using only accessory state.
    #[state]
    accessory_past_user_state_roots: AccessoryStateMap<RollupHeight, [u8; 32]>,

    /// The height of the first DA block.
    /// Set at the rollup genesis. Since the rollup is always delayed by a constant amount of blocks,
    /// we can use this value with the `true_slot_number` to get the current height of the DA layer,
    /// using the following formula:
    /// `current_da_height = true_slot_number + genesis_da_height`.
    /// Should be the same as the `genesis_height` field in the `RunnerConfig` (`sov-stf-runner` crate)
    #[state]
    genesis_da_height: StateValue<u64>,

    /// The rollup's code commitment.
    /// This value is initialized at genesis and can be used to verify the rollup's execution.
    /// This value is used by the `AttesterIncentives` module to verify challenges of attestations.
    #[state]
    inner_code_commitment: StateValue<CodeCommitmentFor<S::InnerZkvm>, BcsCodec>,

    /// Aggregated code commitment.
    /// This value is initialized at genesis and can be used in the aggregated proving circuit to
    /// verify the rollup execution from genesis to the current slot.
    /// This value is used by the `ProverIncentives` module to verify the proofs posted on the DA layer.
    #[state]
    outer_code_commitment: StateValue<CodeCommitmentFor<S::OuterZkvm>, BcsCodec>,
}

impl<S: Spec> ChainState<S> {
    /// Returns the slot number of the current slot using a `BootstrapWorkingSet`. This value is likely
    /// to be stale, because the BootstrapWorkingSet usually only exists at the very start of the state transition before `synchronize_chain` has been called.
    pub fn true_slot_number_at_bootstrap(
        &self,
        state: &mut BootstrapWorkingSet<'_, S>,
    ) -> SlotNumber {
        self.true_slot_number
            .get(state)
            .unwrap_infallible()
            .unwrap_or_default()
    }

    /// Returns the slot number of the current slot using a `BootstrapWorkingSet`. This value is likely
    /// to be stale, because the BootstrapWorkingSet usually only exists at the very start of the state transition before `synchronize_chain` has been called.
    // TODO: Consider exposing this as a custom rest api endpoint
    #[cfg(feature = "native")]
    pub fn true_slot_number_via_api(&self, state: &mut ApiStateAccessor<S>) -> SlotNumber {
        self.true_slot_number
            .get(state)
            .unwrap_infallible()
            .unwrap_or_default()
    }

    /// Returns slot number for the next slot to start execution
    pub fn next_visible_slot_number(
        &self,
        state: &mut BootstrapWorkingSet<'_, S>,
    ) -> VisibleSlotNumber {
        self.next_visible_slot_number
            .get(state)
            .unwrap_infallible()
            .unwrap_or_default()
    }

    /// Returns the visible rollup height corresponding to the provided real slot.
    pub fn visible_slot_number_at<T>(
        &self,
        true_slot_number: SlotNumber,
        state: &mut T,
    ) -> Result<Option<VisibleSlotNumber>, T::Error>
    where
        T: VersionReader + StateReader<Kernel>,
    {
        if true_slot_number == SlotNumber::GENESIS {
            return Ok(Some(VisibleSlotNumber::GENESIS));
        }

        let visible_slot_number = self
            .true_to_visible_slot_number_history
            .get(&true_slot_number, state)?;

        trace!(?visible_slot_number, %true_slot_number, "ChainState::visible_slot_number_at");

        Ok(visible_slot_number)
    }

    /// Returns transition height in the current slot
    pub fn set_next_visible_slot_number(
        &mut self,
        next_visible_slot_number: VisibleSlotNumber,
        state: &mut KernelStateAccessor<S>,
    ) {
        tracing::debug!(%next_visible_slot_number, "Setting next visible slot number");

        self.next_visible_slot_number
            .set(&next_visible_slot_number, state)
            .unwrap_infallible();
    }

    /// Returns the current time, as reported by the DA layer. This can be called within the execution context of a transaction.
    pub fn get_time<Reader: VersionReader + StateReader<Kernel>>(
        &self,
        state: &mut Reader,
    ) -> Result<Time, <Reader as StateReader<Kernel>>::Error> {
        Ok(self
            .time
            .get_current(state)?
            .expect("Time must be set at initialization"))
    }

    /// Return the genesis hash of the module.
    pub fn get_genesis_hash<Accessor: VersionReader + StateReader<Kernel>>(
        &self,
        state: &mut Accessor,
    ) -> Result<Option<<S::Storage as Storage>::Root>, Accessor::Error> {
        Ok(self
            .slots
            .get(&SlotNumber::ONE, state)?
            .map(|slot| slot.prev_state_root))
    }

    /// Return the code commitment to be used for verifying the rollup's execution
    /// for each state transition.
    pub fn inner_code_commitment<Accessor: StateAccessor>(
        &self,
        state: &mut Accessor,
    ) -> Result<Option<CodeCommitmentFor<S::InnerZkvm>>, <Accessor as StateReader<User>>::Error>
    {
        self.inner_code_commitment.get(state)
    }

    /// Return the code commitment to be used for verifying the rollup's execution from genesis to the current slot
    /// in the aggregated proving circuit.
    pub fn outer_code_commitment<Accessor: StateAccessor>(
        &self,
        state: &mut Accessor,
    ) -> Result<Option<CodeCommitmentFor<S::OuterZkvm>>, <Accessor as StateReader<User>>::Error>
    {
        self.outer_code_commitment.get(state)
    }

    /// Return the initial height of the DA layer.
    pub fn genesis_da_height<Accessor: StateAccessor>(
        &self,
        state: &mut Accessor,
    ) -> Result<Option<u64>, <Accessor as StateReader<User>>::Error> {
        self.genesis_da_height.get(state)
    }

    /// Returns the last visible slot processed by the module.
    pub fn latest_visible_slot<Reader: VersionReader + StateReader<Kernel>>(
        &self,
        state: &mut Reader,
    ) -> Result<Option<SlotInformation<S>>, Reader::Error> {
        self.slots.get_current(state)
    }

    /// Returns the "true" last slot processed by the rollup, even if it is not yet visible in user space.
    pub fn kernel_true_latest_slot<Reader: PrivilegedKernelAccessor + StateReader<Kernel>>(
        &self,
        state: &mut Reader,
    ) -> Result<Option<SlotInformation<S>>, <Reader as StateReader<Kernel>>::Error> {
        self.slots.get_true_current(state)
    }

    /// Returns the last root processed by the module.
    pub fn last_root<Reader: VersionReader + StateReader<Kernel>>(
        &self,
        state: &mut Reader,
    ) -> Result<Option<<S::Storage as Storage>::Root>, Reader::Error> {
        Ok(self
            .slots
            .get_current(state)?
            .map(|slot| slot.prev_state_root))
    }

    /// Returns the root hash of the state at the provided height.
    pub fn root_at_height<Accessor: VersionReader + StateReader<Kernel>>(
        &self,
        slot_number: SlotNumber,
        state: &mut Accessor,
    ) -> Result<Option<<S::Storage as Storage>::Root>, Accessor::Error> {
        let Some(next_slot_number) = slot_number.checked_add(1) else {
            return Ok(None);
        };
        Ok(self
            .slots
            .get(&next_slot_number, state)?
            .map(|slot| slot.prev_state_root))
    }

    /// Returns the root hash of the state at the provided height.
    pub fn pre_state_root_at_height<Accessor: VersionReader + StateReader<Kernel>>(
        &self,
        slot_number: SlotNumber,
        state: &mut Accessor,
    ) -> Result<Option<<S::Storage as Storage>::Root>, Accessor::Error> {
        Ok(self
            .slots
            .get(&slot_number, state)?
            .map(|slot| slot.prev_state_root))
    }

    /// Returns the slot information from the state at the provided height.
    pub fn slot_at_height<Accessor: VersionReader + StateReader<Kernel>>(
        &self,
        slot_number: SlotNumber,
        state: &mut Accessor,
    ) -> Result<Option<SlotInformation<S>>, Accessor::Error> {
        self.slots.get(&slot_number, state)
    }

    /// Returns the complete StateTransition for the provided slot number, including the post state root.
    /// Note that this function is marked "dangerous" because it requires the slot *after* the provided slot number to be visible.
    pub fn get_historical_transition_dangerous<Accessor: VersionReader + StateReader<Kernel>>(
        &self,
        slot_number: SlotNumber,
        state: &mut Accessor,
    ) -> Result<Option<StateTransition<S>>, Accessor::Error> {
        let Some(next_slot_num) = slot_number.checked_add(1) else {
            return Ok(None);
        };
        let Some(next_slot) = self.slots.get(&next_slot_num, state)? else {
            return Ok(None);
        };
        let Some(slot) = self.slots.get(&slot_number, state)? else {
            return Ok(None);
        };
        Ok(Some(StateTransition::new(next_slot.prev_state_root, slot)))
    }

    /// Record the gas usage for a given rollup height.
    pub fn record_gas_usage(
        &mut self,
        state: &mut StateCheckpoint<S>,
        final_gas_info: BlockGasInfo<S::Gas>,
        rollup_height: RollupHeight,
    ) {
        self.gas_info
            .set(&rollup_height, &final_gas_info, state)
            .unwrap_infallible();
    }

    /// Returns the current operating mode of the rollup.
    pub fn operating_mode<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<OperatingMode, Accessor::Error> {
        Ok(self
            .operating_mode
            .get(state)?
            .expect("Operating mode must be set at initialization"))
    }

    /// Get the current rollup height
    pub fn rollup_height<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<RollupHeight, Accessor::Error> {
        Ok(self
            .current_heights
            .get(state)?
            .map(|(rollup_height, _)| rollup_height)
            .unwrap_or(RollupHeight::GENESIS))
    }

    /// Returns the visible slot number at the provided height, if that height exists.
    pub fn visible_slot_number_at_height<Accessor: StateReader<User>>(
        &self,
        height: RollupHeight,
        state: &mut Accessor,
    ) -> Result<Option<VisibleSlotNumber>, Accessor::Error> {
        self.slot_number_history.get(&height, state)
    }

    /// Returns the next gas price starting from the provided rollup height using the given visible height increase.
    pub fn compute_next_gas_price<
        Reader: VersionReader + StateReader<User, Error = E> + StateReader<Kernel, Error = E>,
        E,
    >(
        &self,
        stale_rollup_height: RollupHeight,
        provisional_visible_height_increase: u64,
        state: &mut Reader,
    ) -> Result<<<S as Spec>::Gas as Gas>::Price, <Reader as StateReader<Kernel>>::Error> {
        use sov_modules_api::GasSpec;
        if stale_rollup_height.get() == 0 {
            return Ok(S::initial_base_fee_per_gas());
        }
        let prev_gas_info =
            self.gas_info
                .get(&stale_rollup_height, state)?
                .unwrap_or(BlockGasInfo::new(
                    S::initial_gas_limit(),
                    S::initial_base_fee_per_gas(),
                ));

        Ok(Self::compute_base_fee_per_gas(
            prev_gas_info,
            provisional_visible_height_increase,
        ))
    }
}

impl<S: Spec> Module for ChainState<S> {
    type Spec = S;

    type CallMessage = NotInstantiable;

    type Config = ChainStateConfig<S>;

    type Event = ();

    /// Genesis is called when a rollup is deployed and can be used to set initial state values in the module.
    fn genesis(
        &mut self,
        genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<Self::Spec>,
    ) -> anyhow::Result<()> {
        // The initialization logic
        self.init_module(genesis_rollup_header, config, state)
    }

    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: &sov_modules_api::Context<Self::Spec>,
        _state: &mut impl sov_modules_api::TxState<Self::Spec>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

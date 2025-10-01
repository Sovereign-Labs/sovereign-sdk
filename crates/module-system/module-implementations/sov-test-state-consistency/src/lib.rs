#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;

use borsh::{BorshDeserialize, BorshSerialize};
pub use call::*;
mod hooks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_modules_api::{
    AccessoryStateValue, Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo,
    ModuleRestApi, Spec, StateMap, StateValue, TxState,
};
use sov_state::{BorshCodec, Storage};

/// Events emitted by the StateConsistency module
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
    JsonSchema,
)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
pub enum Event<S: Spec> {
    /// Emitted when the value is updated
    ValueUpdated {
        /// The address whose value was updated
        address: S::Address,
        /// The old value that was replaced
        old_value: u64,
        /// The new value that was set
        new_value: u64,
    },
}

/// The State Consistency module. Provides utility transactions for consistency testing through
/// assertions on the state.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct StateConsistency<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    /// Per-account values that get updated
    #[state]
    pub values: StateMap<S::Address, u64, BorshCodec>,

    /// A value that can be set for checking accessory state consistency.
    #[state]
    pub accessory_value: AccessoryStateValue<u64>,

    /// The latest state root stored by the begin slot hook
    #[state]
    pub latest_state_root: StateValue<<<S as Spec>::Storage as Storage>::Root>,
}

impl<S: Spec> Module for StateConsistency<S> {
    type Spec = S;

    type Config = ();

    type CallMessage = call::CallMessage;

    type Event = Event<S>;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.accessory_value.set(&0, state)?;
        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::UpdateValue { old_check, new } => {
                self.update_value(old_check, new, context, state)
            }
            CallMessage::UpdateAccessoryState(new) => {
                self.update_accessory_state(new, context, state)
            }
            CallMessage::AssertBlockState {
                expected_visible_slot_number,
                expected_rollup_height,
                expected_state_root,
            } => self.assert_block_state(
                expected_visible_slot_number,
                expected_rollup_height,
                expected_state_root,
                context,
                state,
            ),
        }
    }
}

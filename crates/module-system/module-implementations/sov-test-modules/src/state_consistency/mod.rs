#![deny(missing_docs)]
#![doc = include_str!("./README.md")]
mod call;

use borsh::{BorshDeserialize, BorshSerialize};
pub use call::*;
mod hooks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_modules_api::{
    AccessoryStateValue, Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo,
    ModuleRestApi, Spec, StateValue, TxState,
};
use sov_state::Storage;

/// Initial configuration for StateConsistency module.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, JsonSchema)]
#[schemars(bound = "S: Spec", rename = "StateConsistencyConfig")]
pub struct StateConsistencyConfig<S: Spec> {
    /// Admin of the module.
    pub admin: S::Address,
}

/// Events emitted by the StateConsistency module
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Emitted when the value is updated
    ValueUpdated {
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

    /// Arbitrary value that gets updated.
    #[state]
    pub value: StateValue<u64>,

    /// A value that can be set for checking accessory state consistency.
    #[state]
    pub accessory_value: AccessoryStateValue<u64>,

    /// The latest state root stored by the begin slot hook
    #[state]
    pub latest_state_root: StateValue<<<S as Spec>::Storage as Storage>::Root>,

    /// The admin address that is allowed to call the module's functions
    #[state]
    pub admin: StateValue<S::Address>,
}

impl<S: Spec> Module for StateConsistency<S> {
    type Spec = S;

    type Config = StateConsistencyConfig<S>;

    type CallMessage = call::CallMessage;

    type Event = Event;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.admin.set(&config.admin, state)?;
        self.value.set(&0, state)?;
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

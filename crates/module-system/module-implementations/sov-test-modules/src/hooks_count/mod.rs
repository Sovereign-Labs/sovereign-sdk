#![deny(missing_docs)]
#![doc = include_str!("./README.md")]
mod call;
mod genesis;

mod event;
pub use call::*;
pub use event::Event;
mod hooks;
use sov_modules_api::{
    AccessoryStateValue, Context, DaSpec, Gas, GenesisState, Module, ModuleId, ModuleInfo,
    ModuleRestApi, Spec, StateValue, TxState,
};
use sov_state::Storage;

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[id]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Can derive ModuleRestApi to automatically generate Rest API endpoints
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct HooksCount<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    /// The number of times the `begin_slot` hook has been called.
    #[state]
    pub begin_rollup_block_hook_count: StateValue<u32>,

    /// The number of times the `end_slot` hook has been called.
    #[state]
    pub end_rollup_block_hook_count: StateValue<u32>,

    /// The number of times the `finalize` hook has been called.
    #[state]
    pub finalize_hook_count: AccessoryStateValue<u32>,

    /// The latest state root stored by the begin slot hook
    #[state]
    pub latest_state_root: StateValue<<<S as Spec>::Storage as Storage>::Root>,
}

/// Gas configuration for the bank module
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash)]
pub struct ValueSeterGasConfig<GU: Gas> {
    /// Gas price multiplier for the set_value operation
    pub set_value: GU,
}

impl<S: Spec> Module for HooksCount<S> {
    type Spec = S;

    type Config = ();

    type CallMessage = call::CallMessage;

    type Event = Event;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        // The initialization logic
        self.init_module(state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::AssertVisibleSlotNumber {
                expected_visible_slot_number,
            } => self.assert_visible_slot_number(expected_visible_slot_number, context, state),
            CallMessage::AssertStateRoot {
                expected_state_root,
            } => self.assert_state_root(expected_state_root, context, state),
            CallMessage::DelayedCallMsg => Ok(()),
        }
    }
}

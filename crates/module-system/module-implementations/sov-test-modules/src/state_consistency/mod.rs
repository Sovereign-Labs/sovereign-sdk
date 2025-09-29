#![deny(missing_docs)]
#![doc = include_str!("./README.md")]
mod call;

pub use call::*;
mod hooks;
use sov_modules_api::{
    AccessoryStateValue, Context, DaSpec, Gas, GenesisState, Module, ModuleId, ModuleInfo,
    ModuleRestApi, Spec, StateValue, TxState,
};
use sov_state::Storage;

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

    /// The value of `value` at the end of a block, set by a hook. Useful for high-throughput soak
    /// testing, to have a value that changes exactly once per block.
    #[state]
    pub value_at_end_of_block: StateValue<u64>,

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

    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.value.set(&0, state)?;
        self.value_at_end_of_block.set(&0, state)?;
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
            CallMessage::UpdateAccessoryState { new } => {
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

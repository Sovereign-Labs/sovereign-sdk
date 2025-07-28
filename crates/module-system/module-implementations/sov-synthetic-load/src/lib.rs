#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod genesis;

mod event;
pub use call::*;
pub use event::Event;
use sov_modules_api::{
    Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, StateValue,
    StateVec, TxState,
};

/// Maximum length for the very large vector used in testing.
pub const VERY_LARGE_VEC_LENGTH: u64 = 1000;

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[id]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Can derive ModuleRestApi to automatically generate Rest API endpoints
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct SyntheticLoad<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    /// A very large vector kept in state initialized with a large number of values.
    #[state]
    pub very_large_vec: StateVec<u64>,

    /// A heavy state kept in state initialized with a large number of values.
    #[state]
    pub heavy_state: StateValue<Vec<u64>>,

    #[phantom]
    _spec: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for SyntheticLoad<S> {
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
        self.init_module(state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let mut state_wrapped = state.to_revertable();
        let state = &mut state_wrapped;
        let res = match msg {
            CallMessage::ReadAndSetManyIndividualValues {
                number_of_operations,
                salt,
            } => Ok(self.read_and_set_many_individual_values(
                number_of_operations,
                salt,
                context,
                state,
            )?),
            CallMessage::ReadAndSetHeavyState {
                number_of_new_values,
                max_heavy_state_size,
                salt,
            } => Ok(self.read_and_set_heavy_state(
                number_of_new_values,
                max_heavy_state_size,
                salt,
                context,
                state,
            )?),
            CallMessage::RunCPUHeavyOperation { iterations } => {
                Ok(self.run_cpu_heavy_operation(iterations, context, state)?)
            }
        };
        state_wrapped.commit();
        res
    }
}

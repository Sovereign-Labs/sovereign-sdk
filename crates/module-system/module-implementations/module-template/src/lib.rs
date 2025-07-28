mod call;
mod event;
mod genesis;
#[cfg(feature = "native")]
mod query;
pub use call::CallMessage;
pub use event::Event;
#[cfg(feature = "native")]
pub use query::*;
use serde::{Deserialize, Serialize};
use sov_modules_api::{
    Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, StateValue,
    TxState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExampleModuleConfig {}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[id]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Can derive ModuleRestApi to automatically generate Rest API endpoints
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct ExampleModule<S: Spec> {
    /// Id of the module.
    #[id]
    pub id: ModuleId,

    /// Some value kept in the state.
    #[state]
    pub value: StateValue<u32>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) _bank: sov_bank::Bank<S>,
}

impl<S: Spec> Module for ExampleModule<S> {
    type Spec = S;

    type Config = ExampleModuleConfig;

    type CallMessage = CallMessage;

    type Event = Event;

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
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::SetValue(new_value) => Ok(self.set_value(new_value, context, state)?),
        }
    }
}

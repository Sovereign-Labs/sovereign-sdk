pub mod call;
pub mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
pub mod query;

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

pub struct ExampleModuleConfig {}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(ModuleInfo)]
pub struct ExampleModule<C: sov_modules_api::Context> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// Some value kept in the state.
    #[state]
    pub value: sov_state::StateValue<u32>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) _bank: bank::Bank<C>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for ExampleModule<C> {
    type Context = C;

    type Config = ExampleModuleConfig;

    type CallMessage = call::CallMessage;

    #[cfg(feature = "native")]
    type QueryMessage = query::QueryMessage;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::SetValue(new_value) => {
                Ok(self.set_value(new_value, context, working_set)?)
            }
        }
    }

    #[cfg(feature = "native")]
    /// This function is on the path to deprecation. We recommend that you leave its body unimplemented
    fn query(
        &self,
        _msg: Self::QueryMessage,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> sov_modules_api::QueryResponse {
        unimplemented!(
            "The query method is on the path to deprecation. Use the RPC macros instead!"
        )
    }
}

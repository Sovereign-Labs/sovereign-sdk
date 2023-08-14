mod call;
mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

pub use call::CallMessage;
#[cfg(feature = "native")]
pub use query::Response;
use sov_modules_api::{Error, ModuleInfo};
use sov_state::WorkingSet;

pub struct ExampleModuleConfig {}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Should derive `ModuleCallJsonSchema` if the "native" feature is enabled.
///   This is optional, and is only used to generate a JSON Schema for your
///   module's call messages (which is useful to develop clients, CLI tooling
///   etc.).
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo)]
pub struct Transfer<C: sov_modules_api::Context> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// Some value kept in the state.
    #[state]
    pub value: sov_state::StateValue<u32>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) _bank: sov_bank::Bank<C>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Transfer<C> {
    type Context = C;

    type Config = ExampleModuleConfig;

    type CallMessage = call::CallMessage;

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
}

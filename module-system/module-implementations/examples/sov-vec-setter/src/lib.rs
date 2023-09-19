#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod genesis;

#[cfg(feature = "native")]
mod query;

pub use call::CallMessage;
#[cfg(feature = "native")]
pub use query::*;
use sov_modules_api::{Error, ModuleInfo, WorkingSet};

/// Initial configuration for sov-vec-setter module.
pub struct VecSetterConfig<C: sov_modules_api::Context> {
    /// Admin of the module.
    pub admin: C::Address,
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo)]
pub struct VecSetter<C: sov_modules_api::Context> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// Some vector kept in the state.
    #[state]
    pub vector: sov_modules_api::StateVec<u32>,

    /// Holds the address of the admin user who is allowed to update the vector.
    #[state]
    pub admin: sov_modules_api::StateValue<C::Address>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for VecSetter<C> {
    type Context = C;

    type Config = VecSetterConfig<C>;

    type CallMessage = call::CallMessage;

    fn genesis(&self, config: &Self::Config, working_set: &mut WorkingSet<C>) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::PushValue(new_value) => {
                Ok(self.push_value(new_value, context, working_set)?)
            }
            call::CallMessage::SetValue { index, value } => {
                Ok(self.set_value(index, value, context, working_set)?)
            }
            call::CallMessage::SetAllValues(values) => {
                Ok(self.set_all_values(values, context, working_set)?)
            }
            call::CallMessage::PopValue => Ok(self.pop_value(context, working_set)?),
        }
    }
}

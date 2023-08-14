#![allow(unused_variables)]
#![allow(dead_code)]

pub mod call;
pub mod genesis;

mod context;
mod router;

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

pub struct ExampleModuleConfig {}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
#[derive(ModuleInfo)]
pub struct IbcModule<C: sov_modules_api::Context> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    #[state]
    pub client_state_store: sov_state::StateMap<String, Vec<u8>>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for IbcModule<C> {
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
        // Note: Here, we would convert into a `MsgEnvelope`, and send to `dispatch()` (i.e. no match statement)
        match msg {
            call::CallMessage::MsgCreateClient(msg) => {
                Ok(self.create_client(msg, context, working_set)?)
            }
        }

        // Q: Do we have to checkpoint the working set here, given that there were no errors?
        // Or is this done by the caller?
        // Similarly for reverting.
    }
}

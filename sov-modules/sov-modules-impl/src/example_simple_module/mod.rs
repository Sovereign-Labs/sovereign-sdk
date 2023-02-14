mod call;
mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

#[cfg(feature = "native")]
use self::query::QueryMessage;

use self::call::CallMessage;
use sov_modules_api::DispatchError;
use sov_modules_macros::ModuleInfo;

#[derive(ModuleInfo)]
pub struct ValueAdderModule<C: sov_modules_api::Context> {
    #[state]
    pub value: sov_state::StateValue<u32, C::Storage>,

    #[state]
    pub admin: sov_state::StateValue<C::PublicKey, C::Storage>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for ValueAdderModule<C> {
    type Context = C;

    type CallMessage = CallMessage;

    #[cfg(feature = "native")]
    type QueryMessage = QueryMessage;

    fn genesis(&mut self) -> Result<(), DispatchError> {
        self.init_module().map_err(|e| e.into())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: Self::Context,
    ) -> Result<sov_modules_api::CallResponse, DispatchError> {
        match msg {
            CallMessage::DoSetValue(set_value) => self
                .set_value(set_value.new_value, context)
                .map_err(|e| e.into()),
        }
    }

    #[cfg(feature = "native")]
    fn query(&self, msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
        match msg {
            QueryMessage::GetValue => self.query_value(),
        }
    }
}

mod call;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

#[cfg(feature = "native")]
use self::query::QueryMessage;

use self::call::CallMessage;
use sov_modules_api::{CallError, QueryError};
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

    type CallMessage = CallMessage<C>;

    #[cfg(feature = "native")]
    type QueryMessage = QueryMessage;

    fn genesis(&mut self) {}

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: Self::Context,
    ) -> Result<sov_modules_api::CallResponse, CallError> {
        match msg {
            CallMessage::DoSetValue(set_value) => self.do_set_value(set_value, context),
            CallMessage::DoAddValue(add_value) => self.do_add_value(add_value.inc),
        }
    }

    #[cfg(feature = "native")]
    fn query(&self, msg: Self::QueryMessage) -> Result<sov_modules_api::QueryResponse, QueryError> {
        match msg {
            QueryMessage::GetValue => self.query_value(),
        }
    }
}

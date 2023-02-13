mod call;
mod query;

use self::{call::CallMessage, query::QueryMessage};
use sov_modules_api::{CallError, QueryError};
use sov_modules_macros::ModuleInfo;
use sovereign_sdk::serial::{Decode, DecodeBorrowed};

#[derive(ModuleInfo)]
pub struct ValueAdderModule<C: sov_modules_api::Context> {
    #[state]
    pub value: sov_state::StateValue<u32, C::Storage>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for ValueAdderModule<C> {
    type Context = C;

    type CallMessage = CallMessage<C>;
    type QueryMessage = QueryMessage;

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

    fn query(&self, msg: Self::QueryMessage) -> Result<sov_modules_api::QueryResponse, QueryError> {
        match msg {
            QueryMessage::GetValue => self.query_value(),
        }
    }
}

//
#[derive(Debug)]
pub struct CustomError {}

// Generated
impl<'de, C: sov_modules_api::Context> DecodeBorrowed<'de> for CallMessage<C> {
    type Error = CustomError;

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}

// Generated
impl<C: sov_modules_api::Context> Decode for CallMessage<C> {
    type Error = CustomError;

    fn decode<R: std::io::Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        todo!()
    }
}

// Generated
impl<'de> DecodeBorrowed<'de> for QueryMessage {
    type Error = CustomError;

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}

// Generated
impl Decode for QueryMessage {
    type Error = CustomError;

    fn decode<R: std::io::Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        todo!()
    }
}

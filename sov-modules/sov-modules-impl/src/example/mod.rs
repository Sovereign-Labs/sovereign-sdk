mod call;
mod query;

use sov_modules_macros::ModuleInfo;
use sovereign_sdk::serial::{Decode, DecodeBorrowed, Encode};

pub struct AccountData {}

#[derive(Debug)]
pub struct CustomError {}

// #[derive(Decode)]
pub struct Transfer<C: sov_modules_api::Context> {
    from: C::PublicKey,
    _to: C::PublicKey,
    _amount: u32,
}

// #[derive(Decode)]
pub struct Delete<C: sov_modules_api::Context> {
    id: C::PublicKey,
}

// #[call_msg]
pub enum CallMessage<C: sov_modules_api::Context> {
    DoTransfer(Transfer<C>),
    DoDeleteAccount(Delete<C>),
}

pub enum CallError {}

impl From<CallError> for sov_modules_api::DecodingError {
    fn from(_: CallError) -> Self {
        todo!()
    }
}

#[derive(ModuleInfo)]
pub struct Bank<C: sov_modules_api::Context> {
    #[state]
    pub accounts: sov_state::StateMap<C::PublicKey, AccountData, C::Storage>,

    #[state]
    pub accounts2: sov_state::StateMap<C::PublicKey, AccountData, C::Storage>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Bank<C> {
    type CallMessage = CallMessage<C>;
    type CallError = CallError;
    type Context = C;

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: Self::Context,
    ) -> Result<sov_modules_api::CallResponse, Self::CallError> {
        match msg {
            CallMessage::DoTransfer(t) => self.do_transfer(t, context),
            CallMessage::DoDeleteAccount(d) => self.do_delete(d, context),
        }
    }
}

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
impl Encode for AccountData {
    fn encode(&self, _target: &mut impl std::io::Write) {
        todo!()
    }
}

impl<'de> DecodeBorrowed<'de> for AccountData {
    type Error = ();

    fn decode_from_slice(target: &'de [u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}

// Generated
impl Decode for AccountData {
    type Error = ();

    fn decode<R: std::io::Read>(target: &mut R) -> Result<Self, <Self as Decode>::Error> {
        todo!()
    }
}

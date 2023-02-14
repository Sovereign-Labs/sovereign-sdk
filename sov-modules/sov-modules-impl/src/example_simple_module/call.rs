use std::fmt::Debug;

use sov_modules_api::{CallResponse, Error};
use sovereign_sdk::serial::{Decode, DecodeBorrowed};

use super::ValueAdderModule;

pub struct SetValue<C: sov_modules_api::Context> {
    from: C::PublicKey,
    new_value: u32,
}

pub enum CallMessage<C: sov_modules_api::Context> {
    DoSetValue(SetValue<C>),
}

#[derive(Debug)]
enum SetValueError {
    BadSender(&'static str),
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub(crate) fn set_value(
        &mut self,
        set_value: SetValue<C>,
        context: C,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        if &set_value.from != context.sender() {
            Err(SetValueError::BadSender("bad sender"))?;
        }

        if set_value.new_value >= 1000 {
            Err("New value should be smaller than 1000")?;
        }

        Ok(CallResponse::default())
    }
}

// Generated
impl<'de, C: sov_modules_api::Context> DecodeBorrowed<'de> for CallMessage<C> {
    type Error = ();

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}

// Generated
impl<C: sov_modules_api::Context> Decode for CallMessage<C> {
    type Error = ();

    fn decode<R: std::io::Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        todo!()
    }
}

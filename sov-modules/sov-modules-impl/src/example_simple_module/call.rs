use std::fmt::Debug;

use sov_modules_api::{CallResponse, ModuleError};
use sovereign_sdk::serial::{Decode, DecodeBorrowed};

use super::ValueAdderModule;

pub struct SetValue {
    pub(crate) new_value: u32,
}

pub enum CallMessage {
    DoSetValue(SetValue),
}

#[derive(Debug)]
enum SetValueError {
    BadSender(&'static str),
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub(crate) fn set_value(
        &mut self,
        new_value: u32,
        context: C,
    ) -> Result<sov_modules_api::CallResponse, ModuleError> {
        let mut response = CallResponse::default();

        let admin = match self.admin.get() {
            Some(admin) => admin,
            None => Err("")?,
        };

        if &admin != context.sender() {
            Err(SetValueError::BadSender("bad sender"))?;
        }

        self.value.set(new_value);
        response.add_event("", "");

        Ok(response)
    }
}

// Generated
impl<'de> DecodeBorrowed<'de> for CallMessage {
    type Error = ();

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}

// Generated
impl Decode for CallMessage {
    type Error = ();

    fn decode<R: std::io::Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        todo!()
    }
}

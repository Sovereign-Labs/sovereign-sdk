use std::fmt::Debug;

use sov_modules_api::{CallError, CallResponse};

use super::ValueAdderModule;

pub struct SetValue<C: sov_modules_api::Context> {
    from: C::PublicKey,
    new_value: u32,
}

pub struct AddValue {
    pub(crate) inc: u32,
}

pub enum CallMessage<C: sov_modules_api::Context> {
    DoSetValue(SetValue<C>),
    DoAddValue(AddValue),
}

#[derive(Debug)]
enum SetValueError {
    BadSender(&'static str),
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub(crate) fn do_set_value(
        &mut self,
        set_value: SetValue<C>,
        context: C,
    ) -> Result<sov_modules_api::CallResponse, CallError> {
        if &set_value.from != context.sender() {
            Err(SetValueError::BadSender("bad sender"))?;
        }

        if set_value.new_value >= 1000 {
            Err("New value should be smaller than 1000")?;
        }

        Ok(CallResponse::default())
    }

    pub(crate) fn do_add_value(
        &mut self,
        inc: u32,
    ) -> Result<sov_modules_api::CallResponse, CallError> {
        let mut response = CallResponse::default();

        let old_value = match self.value.get() {
            Some(olad_value) => olad_value,
            None => todo!(),
        };

        let new_value = old_value + inc;
        self.value.set(new_value);

        response.add_event("key", "value");

        Ok(response)
    }
}

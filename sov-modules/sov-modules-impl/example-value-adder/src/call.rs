use anyhow::Result;
use borsh::BorshDeserialize;
use sov_modules_api::CallResponse;
use std::fmt::Debug;
use thiserror::Error;

use super::ValueAdderModule;

#[derive(BorshDeserialize)]
pub struct SetValue {
    pub(crate) new_value: u32,
}

#[derive(BorshDeserialize)]
pub enum CallMessage {
    DoSetValue(SetValue),
}

#[derive(Debug, Error)]
enum SetValueError {
    #[error("Only admin can change the value")]
    WrongSender,
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    /// Sets `value` field to the `new_value`, only admin is authorized to call this method.
    pub(crate) fn set_value(
        &mut self,
        new_value: u32,
        context: &C,
    ) -> Result<sov_modules_api::CallResponse> {
        let mut response = CallResponse::default();

        let admin = self.admin.get_or_err()?;

        if &admin != context.sender() {
            // Here we use a custom error type.
            Err(SetValueError::WrongSender)?;
        }

        self.value.set(new_value);
        response.add_event("add_event", &format!("value_set: {new_value:?}"));

        Ok(response)
    }
}

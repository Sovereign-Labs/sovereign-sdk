use std::fmt::Debug;

use anyhow::Result;
#[cfg(feature = "native")]
use sov_modules_api::macros::CliWalletArg;
use sov_modules_api::{CallResponse, WorkingSet};
use thiserror::Error;

use super::ValueSetter;

/// This enumeration represents the available call messages for interacting with the `sov-value-setter` module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(CliWalletArg),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage {
    /// value to set
    SetValue(u32),
}

/// Example of a custom error.
#[derive(Debug, Error)]
enum SetValueError {
    #[error("Only admin can change the value")]
    WrongSender,
}

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Sets `value` field to the `new_value`, only admin is authorized to call this method.
    pub(crate) fn set_value(
        &self,
        new_value: u32,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> Result<sov_modules_api::CallResponse> {
        // If admin is not then early return:
        let admin = self.admin.get_or_err(working_set)?;

        if &admin != context.sender() {
            // Here we use a custom error type.
            Err(SetValueError::WrongSender)?;
        }

        // This is how we set a new value:
        self.value.set(&new_value, working_set);
        working_set.add_event("set", &format!("value_set: {new_value:?}"));

        Ok(CallResponse::default())
    }
}

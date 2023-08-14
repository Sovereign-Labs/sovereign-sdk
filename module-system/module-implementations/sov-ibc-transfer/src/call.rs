use std::fmt::Debug;

use anyhow::Result;
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;
use thiserror::Error;

use crate::Transfer;

/// This enumeration represents the available call messages for interacting with
/// the `ExampleModule` module.
/// The `derive` for [`schemars::JsonSchema`] is a requirement of
/// [`sov_modules_api::ModuleCallJsonSchema`].
#[cfg_attr(feature = "native", derive(schemars::JsonSchema))]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage {
    SetValue(u32),
}

/// Example of a custom error.
#[derive(Debug, Error)]
enum SetValueError {}

impl<C: sov_modules_api::Context> Transfer<C> {
    /// Sets `value` field to the `new_value`
    pub(crate) fn set_value(
        &self,
        new_value: u32,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        self.value.set(&new_value, working_set);
        working_set.add_event("set", &format!("value_set: {new_value:?}"));

        Ok(CallResponse::default())
    }
}

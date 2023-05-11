use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;
use std::fmt::Debug;
use thiserror::Error;

use crate::ExampleModule;

/// This enumeration represents the available call messages for interacting with the `ExampleModule` module.
#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum CallMessage {
    SetValue(u32),
}

/// Example of a custom error.
#[derive(Debug, Error)]
enum SetValueError {}

impl<C: sov_modules_api::Context> ExampleModule<C> {
    /// Sets `value` field to the `new_value`
    pub(crate) fn set_value(
        &self,
        new_value: u32,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        let mut response = CallResponse::default();

        self.value.set(new_value, working_set);
        response.add_event("set", &format!("value_set: {new_value:?}"));

        Ok(response)
    }
}

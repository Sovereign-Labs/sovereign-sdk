use std::fmt::Debug;

use anyhow::Result;
#[cfg(feature = "native")]
use sov_modules_api::macros::CliWalletArg;
use sov_modules_api::{CallResponse, WorkingSet};
use thiserror::Error;

use super::VecSetter;

/// This enumeration represents the available call messages for interacting with the `sov-vec-setter` module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(CliWalletArg),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage {
    /// value to push
    PushValue(u32),
    /// value to set
    SetValue {
        /// index to set
        index: usize,
        /// value to set
        value: u32,
    },
    /// values to set
    SetAllValues(Vec<u32>),
    /// Pop
    PopValue,
}

/// Example of a custom error.
#[derive(Debug, Error)]
enum SetValueError {
    #[error("Only admin can change the value")]
    WrongSender,
}

impl<C: sov_modules_api::Context> VecSetter<C> {
    /// Pushes `value` field to the `vector`, only admin is authorized to call this method.
    pub(crate) fn push_value(
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

        // This is how we push a new value to vector:
        self.vector.push(&new_value, working_set);

        let new_length = self.vector.len(working_set);

        working_set.add_event(
            "push",
            &format!("value_push: {new_value:?}, new length: {new_length:?}"),
        );

        Ok(CallResponse::default())
    }

    /// Sets `value` field to the given index of `vector`, only admin is authorized to call this method.
    pub(crate) fn set_value(
        &self,
        index: usize,
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
        self.vector.set(index, &new_value, working_set)?;

        working_set.add_event(
            "set",
            &format!("value_set: {new_value:?} for index: {index:?}"),
        );

        Ok(CallResponse::default())
    }

    /// Sets `values` completely to the `vector`, only admin is authorized to call this method.
    pub(crate) fn set_all_values(
        &self,
        values: Vec<u32>,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> Result<sov_modules_api::CallResponse> {
        // If admin is not then early return:
        let admin = self.admin.get_or_err(working_set)?;

        if &admin != context.sender() {
            // Here we use a custom error type.
            Err(SetValueError::WrongSender)?;
        }

        // This is how we set all the vector:
        self.vector.set_all(values, working_set);

        let new_length = self.vector.len(working_set);

        working_set.add_event("set_all", &format!("new length: {new_length:?}"));

        Ok(CallResponse::default())
    }

    /// Pops last value from the `vector`, only admin is authorized to call this method.
    pub(crate) fn pop_value(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> Result<sov_modules_api::CallResponse> {
        // If admin is not then early return:
        let admin = self.admin.get_or_err(working_set)?;

        if &admin != context.sender() {
            // Here we use a custom error type.
            Err(SetValueError::WrongSender)?;
        }

        // This is how we pop last value value:
        let pop_value = self.vector.pop(working_set);

        let new_length = self.vector.len(working_set);

        working_set.add_event(
            "pop",
            &format!("value_pop: {pop_value:?}, new length: {new_length:?}"),
        );

        Ok(CallResponse::default())
    }
}

use std::fmt::Debug;

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
}

use std::fmt::Debug;

use ibc::applications::transfer::msgs::transfer::MsgTransfer;
use thiserror::Error;

use crate::Transfer;

/// This enumeration represents the available call messages for interacting with
/// the `ExampleModule` module.
/// The `derive` for [`schemars::JsonSchema`] is a requirement of
/// [`sov_modules_api::ModuleCallJsonSchema`].
#[cfg_attr(
    feature = "native",
    derive(schemars::JsonSchema),
    schemars(bound = "C::Address: ::schemars::JsonSchema", rename = "CallMessage")
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct CallMessage<C: sov_modules_api::Context> {
    pub msg_transfer: MsgTransfer,
    pub token_address: C::Address,
}

/// Example of a custom error.
#[derive(Debug, Error)]
enum SetValueError {}

impl<C: sov_modules_api::Context> Transfer<C> {}

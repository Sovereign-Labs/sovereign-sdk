#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
pub mod query;

use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo};
use sov_state::{AccessoryStateValue, StateValue, WorkingSet};

/// [`AccessorySetter`] is a module that stores data both *inside* the JMT and
/// *outside* the JMT.
///
/// Data stored inside the JMT contributes to the state root hash, and is always
/// accessible. This costs significant compute, and should be avoided for all
/// data that is not necessary to the core functioning of the rollup. Other data
/// that facilitates serving queries over JSON-RPC or only accessed by tooling
/// doesn't need to be verifiable, and can thus be stored outside the JMT much
/// more cheaply. Since accessory data is not included in the state root hash,
/// it is not accessible inside the zkVM and can only be accessed with
/// `#[cfg(feature = "native")]`.
#[derive(ModuleInfo)]
pub struct AccessorySetter<C: sov_modules_api::Context> {
    /// The address of the module.
    #[address]
    pub address: C::Address,
    /// Some arbitrary value stored in the JMT to demonstrate the difference
    /// between the JMT and accessory state.
    #[state]
    pub state_value: StateValue<String>,
    /// A non-JMT value stored in the accessory state.
    #[state]
    pub accessory_value: AccessoryStateValue<String>,
}

/// The [`Module::CallMessage`] for [`AccessorySetter`].
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage {
    /// Sets the value of [`AccessorySetter::state_value`].
    SetValue(String),
    /// Stores some arbitrary value in the accessory state.
    SetValueAccessory(String),
}

impl<C: Context> Module for AccessorySetter<C> {
    type Context = C;

    type Config = ();

    type CallMessage = CallMessage;

    fn call(
        &self,
        msg: Self::CallMessage,
        _context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            CallMessage::SetValueAccessory(new_value) => {
                self.accessory_value
                    .set(&new_value, &mut working_set.accessory_state());
            }
            CallMessage::SetValue(new_value) => {
                self.state_value.set(&new_value, working_set);
            }
        };
        Ok(CallResponse::default())
    }
}

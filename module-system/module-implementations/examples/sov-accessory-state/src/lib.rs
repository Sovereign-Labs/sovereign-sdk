#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
pub mod query;

use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo};
use sov_state::codec::BorshCodec;
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{Prefix, StateValue, WorkingSet};

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
    pub latest_value: StateValue<String>,
}

/// The [`Module::CallMessage`] for [`AccessorySetter`].
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage {
    /// Sets the value of [`AccessorySetter::latest_value`].
    SetValue(String),
    /// Stores some arbitrary value in the accessory state.
    SetValueAccessory(String),
}

impl<C: Context> AccessorySetter<C> {
    /// Sets a value in the JMT state.
    fn set_value(&self, s: String, working_set: &mut WorkingSet<C::Storage>) {
        self.latest_value.set(&s, working_set);
    }

    /// Sets a value in the accessory state.
    fn set_value_accessory(&self, s: String, working_set: &mut WorkingSet<C::Storage>) {
        let prefix = Prefix::new(self.address.as_ref().to_vec());
        let key = StorageKey::new(&prefix, "value");
        let new_value = StorageValue::new(&s.into_bytes(), &BorshCodec);
        working_set.set_accessory(key, new_value);
    }

    /// Returns the value of [`AccessorySetter::latest_value`].
    pub fn get_value(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<String> {
        self.latest_value.get(working_set)
    }

    /// Returns the latest value set in the accessory state via
    /// [`CallMessage::SetValueAccessory`].
    #[cfg(feature = "native")]
    pub fn get_value_accessory(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<String> {
        use sov_state::codec::StateValueCodec;

        let prefix = Prefix::new(self.address.as_ref().to_vec());
        let key = StorageKey::new(&prefix, "value");
        let storage_value = working_set.get_accessory(key);
        storage_value.and_then(|v| BorshCodec.try_decode_value(v.value()).ok())
    }
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
                self.set_value_accessory(new_value, working_set);
            }
            CallMessage::SetValue(new_value) => {
                self.set_value(new_value, working_set);
            }
        };
        Ok(CallResponse::default())
    }
}

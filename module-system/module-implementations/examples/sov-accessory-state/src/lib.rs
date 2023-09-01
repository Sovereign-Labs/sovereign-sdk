#[cfg(feature = "native")]
pub mod query;

use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo};
use sov_state::codec::BorshCodec;
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{Prefix, StateValue, WorkingSet};

#[derive(ModuleInfo)]
pub struct AccessorySetter<C: sov_modules_api::Context> {
    #[address]
    pub address: C::Address,
    #[state]
    pub latest_value: StateValue<String>,
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage {
    SetValue(String),
    SetValueAccessory(String),
}

impl<C: Context> AccessorySetter<C> {
    /// Sets a value in the JMT state.
    fn set_value(&self, s: String, working_set: &mut WorkingSet<C::Storage>) {
        self.latest_value.set(&s, working_set);
    }

    /// Sets a value in the non-JMT state.
    fn set_value_accessory(&self, s: String, working_set: &mut WorkingSet<C::Storage>) {
        let prefix = Prefix::new(self.address.as_ref().to_vec());
        let key = StorageKey::new(&prefix, "value");
        let new_value = StorageValue::new(&s.into_bytes(), &BorshCodec);
        working_set.set_accessory(key, new_value);
    }

    pub fn get_value(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<String> {
        self.latest_value.get(working_set)
    }

    #[cfg(feature = "native")]
    pub fn get_value_accessory(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<String> {
        use sov_state::codec::StateValueCodec;

        let prefix = Prefix::new(self.address.as_ref().to_vec());
        let key = StorageKey::new(&prefix, "value");
        let storage_value = working_set.get_accessory(key);
        let value = storage_value.and_then(|v| BorshCodec.try_decode_value(v.value()).ok());
        value
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

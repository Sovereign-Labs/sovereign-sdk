use crate::{storage::StorageKey, Prefix, StateMap, Storage};
use sovereign_sdk::serial::{Decode, Encode};
use std::marker::PhantomData;

#[derive(Debug)]
pub struct StateValue<V, S> {
    _phantom: PhantomData<V>,
    // TODO comment
    map: StateMap<(), V, S>,
}

impl<V: Encode + Decode, S: Storage> StateValue<V, S> {
    pub fn new(storage: S, prefix: Prefix) -> Self {
        Self {
            _phantom: PhantomData,
            map: StateMap::new(storage, prefix),
        }
    }

    pub fn set(&mut self, value: V) {
        let storage_key = StorageKey::new_with_empty_state_key(self.map.prefix());
        self.map.set_value(storage_key, value)
    }

    pub fn get(&self) -> Option<V> {
        let storage_key = StorageKey::new_with_empty_state_key(self.map.prefix());
        self.map.get_value(storage_key)
    }

    pub fn prefix(&self) -> &Prefix {
        self.map.prefix()
    }
}

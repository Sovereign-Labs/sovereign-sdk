use crate::{storage::StorageKey, Prefix, StateMap, Storage};
use sovereign_sdk::serial::{Decode, Encode};
use std::marker::PhantomData;

/// SingletonKey is very similar to the unit type `()` i.e. it has only one value.
/// We provide a custom efficient Encode implementation for SingletonKey while Encode for `()`
/// is likely already implemented by an external library (like borsh) which is outside of our control.
#[derive(Debug)]
pub struct SingletonKey;

impl Encode for SingletonKey {
    fn encode(&self, _: &mut impl std::io::Write) {
        // Do nothing.
    }
}

/// Container for a single value.
#[derive(Debug)]
pub struct StateValue<V, S> {
    _phantom: PhantomData<V>,
    // StateValue is equivalent to a StateMap with a single key.
    map: StateMap<SingletonKey, V, S>,
}

impl<V: Encode + Decode, S: Storage> StateValue<V, S> {
    pub fn new(storage: S, prefix: Prefix) -> Self {
        Self {
            _phantom: PhantomData,
            map: StateMap::new(storage, prefix),
        }
    }

    /// Sets a value in the StateValue.
    pub fn set(&mut self, value: V) {
        // `StorageKey::new` will serialize the SingletonKey, but that's fine because we provided
        //  efficient Encode implementation.
        let storage_key = StorageKey::new(self.map.prefix(), SingletonKey);
        self.map.set_value(storage_key, value)
    }

    /// Gets a value from the StateValue.
    pub fn get(&self) -> Option<V> {
        let storage_key = StorageKey::new(self.map.prefix(), SingletonKey);
        self.map.get_value(storage_key)
    }

    pub fn prefix(&self) -> &Prefix {
        self.map.prefix()
    }
}

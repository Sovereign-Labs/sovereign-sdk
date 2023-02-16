use crate::{backend::Backend, storage::StorageKey, Prefix, Storage};
use sovereign_sdk::serial::{Decode, Encode};

// SingletonKey is very similar to the unit type `()` i.e. it has only one value.
// We provide a custom efficient Encode implementation for SingletonKey while Encode for `()`
// is likely already implemented by an external library (like borsh), which is outside of our control.
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
    // StateValue is equivalent to a Backend with a single key.
    backend: Backend<SingletonKey, V, S>,
}

impl<V: Encode + Decode, S: Storage> StateValue<V, S> {
    pub fn new(storage: S, prefix: Prefix) -> Self {
        Self {
            backend: Backend::new(storage, prefix),
        }
    }

    /// Sets a value in the StateValue.
    pub fn set(&mut self, value: V) {
        // `StorageKey::new` will serialize the SingletonKey, but that's fine because we provided
        //  efficient Encode implementation.
        let storage_key = StorageKey::new(self.backend.prefix(), &SingletonKey);
        self.backend.set_value(storage_key, value)
    }

    /// Gets a value from the StateValue.
    pub fn get(&self) -> Option<V> {
        let storage_key = StorageKey::new(self.backend.prefix(), &SingletonKey);
        self.backend.get_value(storage_key)
    }

    pub fn prefix(&self) -> &Prefix {
        self.backend.prefix()
    }
}

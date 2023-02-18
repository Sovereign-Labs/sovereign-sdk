use crate::{backend::Backend, storage::StorageKey, Prefix, Storage};
use sovereign_sdk::serial::{Decode, Encode};
use thiserror::Error;

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

/// Error type for `StateValue` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
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

    /// Gets a value from the StateValue or None if the value is absent.
    pub fn get(&self) -> Option<V> {
        let storage_key = StorageKey::new(self.backend.prefix(), &SingletonKey);
        self.backend.get_value(storage_key)
    }

    /// Gets a value from the StateValue or Error if the value is absent.
    pub fn get_or_err(&self) -> Result<V, Error> {
        self.get().ok_or(Error::MissingValue(self.prefix().clone()))
    }

    pub fn prefix(&self) -> &Prefix {
        self.backend.prefix()
    }
}

use std::marker::PhantomData;

use crate::{storage::StorageKey, Prefix, Storage, WorkingSet};
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
pub struct StateValue<V, S: Storage> {
    _phantom: (PhantomData<V>, PhantomData<S>),
    prefix: Prefix,
}

/// Error type for `StateValue` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
}

impl<V: Encode + Decode, S: Storage> StateValue<V, S> {
    pub fn new(prefix: Prefix) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            prefix,
        }
    }

    /// Sets a value in the StateValue.
    pub fn set(&mut self, value: V, working_set: &mut WorkingSet<S>) {
        // `StorageKey::new` will serialize the SingletonKey, but that's fine because we provided
        //  efficient Encode implementation.
        let storage_key = StorageKey::new(self.prefix(), &SingletonKey);
        working_set.set_value(storage_key, value)
    }

    /// Gets a value from the StateValue or None if the value is absent.
    pub fn get(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
        let storage_key = StorageKey::new(self.prefix(), &SingletonKey);
        working_set.get_value(storage_key)
    }

    /// Gets a value from the StateValue or Error if the value is absent.
    pub fn get_or_err(&self, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.get(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    // Removes a value from the StateValue, returning the value (or None if the key is absent).
    pub fn remove(&mut self, working_set: &mut WorkingSet<S>) -> Option<V> {
        let storage_key = StorageKey::new(self.prefix(), &SingletonKey);
        working_set.remove_value(storage_key)
    }

    // Removes a value and from the StateValue, returning the value (or Error if the key is absent).
    pub fn remove_or_err(&mut self, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.remove(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

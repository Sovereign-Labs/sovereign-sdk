use std::io::Write;
use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use thiserror::Error;

use crate::{Prefix, Storage, WorkingSet};

// SingletonKey is very similar to the unit type `()` i.e. it has only one value.
#[derive(Debug, BorshDeserialize)]
pub struct SingletonKey;

impl BorshSerialize for SingletonKey {
    fn serialize<W: Write>(&self, _writer: &mut W) -> std::io::Result<()> {
        Ok(())
    }

    fn try_to_vec(&self) -> std::io::Result<Vec<u8>> {
        Ok(vec![])
    }
}

/// Container for a single value.
#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize)]
pub struct StateValue<V> {
    _phantom: PhantomData<V>,
    prefix: Prefix,
}

/// Error type for `StateValue` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
}

impl<V: BorshSerialize + BorshDeserialize> StateValue<V> {
    pub fn new(prefix: Prefix) -> Self {
        Self {
            _phantom: PhantomData,
            prefix,
        }
    }

    /// Sets a value in the StateValue.
    pub fn set<S: Storage>(&self, value: &V, working_set: &mut WorkingSet<S>) {
        working_set.set_value(self.prefix(), &SingletonKey, value)
    }

    /// Gets a value from the StateValue or None if the value is absent.
    pub fn get<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
        working_set.get_value(self.prefix(), &SingletonKey)
    }

    /// Gets a value from the StateValue or Error if the value is absent.
    pub fn get_or_err<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.get(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Removes a value from the StateValue, returning the value (or None if the key is absent).
    pub fn remove<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
        working_set.remove_value(self.prefix(), &SingletonKey)
    }

    /// Removes a value and from the StateValue, returning the value (or Error if the key is absent).
    pub fn remove_or_err<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.remove(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Deletes a value from the StateValue.
    pub fn delete<S: Storage>(&self, working_set: &mut WorkingSet<S>) {
        working_set.delete_value(self.prefix(), &SingletonKey);
    }

    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

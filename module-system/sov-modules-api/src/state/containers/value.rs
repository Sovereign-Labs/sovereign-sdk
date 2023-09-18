use std::marker::PhantomData;

use sov_state::codec::{BorshCodec, StateCodec, StateValueCodec};
use sov_state::Prefix;
use thiserror::Error;

use crate::state::{StateReaderAndWriter, WorkingSet};
use crate::Context;

/// Container for a single value.
#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct StateValue<V, Codec = BorshCodec> {
    _phantom: PhantomData<V>,
    codec: Codec,
    prefix: Prefix,
}

/// Error type for `StateValue` get method.
#[derive(Debug, Error)]
pub enum Error {
    /// Value not found.
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
}

impl<V> StateValue<V> {
    /// Crates a new [`StateValue`] with the given prefix and the default
    /// [`StateValueCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self::with_codec(prefix, BorshCodec)
    }
}

impl<V, Codec> StateValue<V, Codec> {
    /// Creates a new [`StateValue`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            _phantom: PhantomData,
            codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`StateValue`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

impl<V, Codec> StateValue<V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateValueCodec<V>,
{
    /// Sets a value in the StateValue.
    pub fn set<C: Context>(&self, value: &V, working_set: &mut WorkingSet<C>) {
        working_set.set_singleton(self.prefix(), value, &self.codec)
    }

    /// Gets a value from the StateValue or None if the value is absent.
    pub fn get<C: Context>(&self, working_set: &mut WorkingSet<C>) -> Option<V> {
        working_set.get_singleton(self.prefix(), &self.codec)
    }

    /// Gets a value from the StateValue or Error if the value is absent.
    pub fn get_or_err<C: Context>(&self, working_set: &mut WorkingSet<C>) -> Result<V, Error> {
        self.get(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Removes a value from the StateValue, returning the value (or None if the key is absent).
    pub fn remove<C: Context>(&self, working_set: &mut WorkingSet<C>) -> Option<V> {
        working_set.remove_singleton(self.prefix(), &self.codec)
    }

    /// Removes a value and from the StateValue, returning the value (or Error if the key is absent).
    pub fn remove_or_err<C: Context>(&self, working_set: &mut WorkingSet<C>) -> Result<V, Error> {
        self.remove(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Deletes a value from the StateValue.
    pub fn delete<C: Context>(&self, working_set: &mut WorkingSet<C>) {
        working_set.delete_singleton(self.prefix());
    }
}

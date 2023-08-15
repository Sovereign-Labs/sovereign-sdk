use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use thiserror::Error;

use crate::codec::{BorshCodec, StateKeyCodec, StateKeyEncode, StateValueCodec};
use crate::{Prefix, Storage, WorkingSet};

/// Container for a single value.
#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize)]
pub struct StateValue<V, C = BorshCodec> {
    _phantom: PhantomData<V>,
    codec: C,
    prefix: Prefix,
}

/// Error type for `StateValue` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
}

impl<V> StateValue<V>
where
    BorshCodec: StateValueCodec<V>,
{
    /// Crates a new [`StateValue`] with the given prefix and the default
    /// [`StateCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self {
            _phantom: PhantomData,
            codec: BorshCodec,
            prefix,
        }
    }
}

impl<V, C> StateValue<V, C>
where
    C: StateValueCodec<V>,
{
    /// Creates a new [`StateValue`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: C) -> Self {
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

    fn internal_codec(&self) -> SingletonCodec<C> {
        SingletonCodec::new(&self.codec)
    }

    /// Sets a value in the StateValue.
    pub fn set<S: Storage>(&self, value: &V, working_set: &mut WorkingSet<S>) {
        working_set.set_value(self.prefix(), &self.internal_codec(), &SingletonKey, value)
    }

    /// Gets a value from the StateValue or None if the value is absent.
    pub fn get<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
        working_set.get_value(self.prefix(), &self.internal_codec(), &SingletonKey)
    }

    /// Gets a value from the StateValue or Error if the value is absent.
    pub fn get_or_err<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.get(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Removes a value from the StateValue, returning the value (or None if the key is absent).
    pub fn remove<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
        working_set.remove_value(self.prefix(), &self.internal_codec(), &SingletonKey)
    }

    /// Removes a value and from the StateValue, returning the value (or Error if the key is absent).
    pub fn remove_or_err<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.remove(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Deletes a value from the StateValue.
    pub fn delete<S: Storage>(&self, working_set: &mut WorkingSet<S>) {
        working_set.delete_value(self.prefix(), &self.internal_codec(), &SingletonKey);
    }
}

// SingletonKey is very similar to the unit type `()` i.e. it has only one value.
#[derive(Debug)]
struct SingletonKey;

/// Skips (de)serialization of keys and delegates values to another codec.
struct SingletonCodec<'a, VC> {
    value_codec: &'a VC,
}

impl<'a, VC> SingletonCodec<'a, VC> {
    pub fn new(value_codec: &'a VC) -> Self {
        Self { value_codec }
    }
}

impl<'a, VC> StateKeyEncode<SingletonKey> for SingletonCodec<'a, VC> {
    fn encode_key(&self, _: &SingletonKey) -> Vec<u8> {
        vec![]
    }
}

impl<'a, VC> StateKeyCodec<SingletonKey> for SingletonCodec<'a, VC> {
    type KeyError = std::io::Error;

    fn try_decode_key(&self, bytes: &[u8]) -> Result<SingletonKey, Self::KeyError> {
        if bytes.is_empty() {
            Ok(SingletonKey)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "SingletonKey must be empty",
            ))
        }
    }
}

impl<'a, V, VC> StateValueCodec<V> for SingletonCodec<'a, VC>
where
    VC: StateValueCodec<V>,
{
    type ValueError = VC::ValueError;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        self.value_codec.encode_value(value)
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::ValueError> {
        self.value_codec.try_decode_value(bytes)
    }
}

use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use thiserror::Error;

use crate::codec::{BorshCodec, StateKeyCodec, StateValueCodec};
use crate::{Prefix, Storage, WorkingSet};

#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize)]
pub struct StateVec<V, C = BorshCodec> {
    _phantom: PhantomData<V>,
    codec: C,
    prefix: Prefix,
}

/// Error type for `StateVec` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Index out of bounds: {0}")]
    IndexOutOfBounds(usize),
}

impl<V> StateVec<V>
where
    BorshCodec: StateValueCodec<V>,
{
    /// Crates a new [`StateVec`] with the given prefix and the default
    /// [`StateCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self {
            _phantom: PhantomData,
            codec: BorshCodec,
            prefix,
        }
    }
}

impl<V, C> StateVec<V, C>
where
    C: StateValueCodec<V>,
    C: StateValueCodec<usize>,
{
    /// Creates a new [`StateVec`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: C) -> Self {
        Self {
            _phantom: PhantomData,
            codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`StateVec`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    fn internal_codec(&self) -> IndexCodec<C> {
        IndexCodec::new(&self.codec)
    }

    /// Sets a value in the [`StateVec`].
    /// If the index is out of bounds, returns an error.
    /// To push a value to the end of the StateVec, use [`StateVec::push`].
    pub fn set<S: Storage>(&self, index: usize, value: &V, working_set: &mut WorkingSet<S>) -> Result<(), Error> {
        let len = self.len(working_set);

        if index < len {
            working_set.set_value(self.prefix(), &self.internal_codec(), &IndexKey(index + 1), value);
            Ok(())
        } else {
            Err(Error::IndexOutOfBounds(index))
        }
    }

    /// Returns the value for the given index.
    pub fn get<S: Storage>(&self, index: usize, working_set: &mut WorkingSet<S>) -> Result<Option<V>, Error> {
        let len = self.len(working_set);

        if index < len {
            let elem = working_set.get_value(self.prefix(), &self.internal_codec(), &IndexKey(index + 1));

            Ok(elem)
        } else {
            Err(Error::IndexOutOfBounds(index))
        }
    }

    /// Returns the length of the [`StateVec`].
    pub fn len<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> usize {
        let len = working_set.get_value::<_, usize, _>(self.prefix(), &self.internal_codec(), &IndexKey(0));
        len.unwrap_or_default()
    }

    fn set_len<S: Storage>(&self, length: usize, working_set: &mut WorkingSet<S>) {
        working_set.set_value(self.prefix(), &self.internal_codec(), &IndexKey(0), &length);
    }

    /// Pushes a value to the end of the [`StateVec`].
    pub fn push<S: Storage>(&self, value: &V, working_set: &mut WorkingSet<S>) {
        let len = self.len(working_set);

        working_set.set_value(self.prefix(), &self.internal_codec(), &IndexKey(len + 1), value);
        self.set_len(len + 1, working_set);
    }

    /// Pops a value from the end of the [`StateVec`] and returns it.
    pub fn pop<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
        let len = self.len(working_set);

        if len > 0 {
            let elem = working_set.remove_value(self.prefix(), &self.internal_codec(), &IndexKey(len));
            self.set_len(len - 1, working_set);
            elem
        } else {
            None
        }
    }

    /// Sets all values in the [`StateVec`].
    /// If the length of the provided values is less than the length of the [`StateVec`], the remaining values stay in storage but are inaccessible.
    pub fn set_all<S: Storage>(&self, values: Vec<V>, working_set: &mut WorkingSet<S>) {
        let len = self.len(working_set);

        let new_len = values.len(); 

        for (i, value) in values.into_iter().enumerate() {
            if i < len {
                let _ = self.set(i, &value, working_set);
            } else {
                self.push(&value, working_set);
            }
        }

        // if new_len > len push() already handles setting length
        if new_len < len {
            self.set_len(new_len, working_set);
        }
    }
}

#[derive(Debug)]
struct IndexKey(usize);

struct IndexCodec<'a, VC> {
    value_codec: &'a VC,
}

impl<'a, VC> IndexCodec<'a, VC> {
    pub fn new(value_codec: &'a VC) -> Self {
        Self { value_codec }
    }
}

impl<'a, VC> StateKeyCodec<IndexKey> for IndexCodec<'a, VC> {
    type KeyError = std::io::Error;

    fn encode_key(&self, i: &IndexKey) -> Vec<u8> {
        i.0.to_be_bytes().to_vec()
    }

    fn try_decode_key(&self, bytes: &[u8]) -> Result<IndexKey, Self::KeyError> {
        if bytes.is_empty() {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IndexKey must not be empty",
            ))
        } else {
            Ok(IndexKey(
                usize::from_be_bytes(bytes.try_into().expect("Couldn't cast to [u8; 8]"))
            ))
        }
    }
}

impl<'a, V, VC> StateValueCodec<V> for IndexCodec<'a, VC>
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
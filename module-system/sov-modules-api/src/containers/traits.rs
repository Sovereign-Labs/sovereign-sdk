use std::iter::FusedIterator;

use sov_modules_core::{
    EncodeKeyLike, Prefix, StateCodec, StateKeyCodec, StateReaderAndWriter, StateValueCodec,
    StorageKey,
};
use thiserror::Error;

/// Error type for `StateValue` get method.
#[derive(Debug, Error)]
pub enum StateValueError {
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
}

/// Error type for the [`StateMap::get`] method.
#[derive(Debug, Error)]
pub enum StateMapError {
    /// Value not found.
    #[error("Value not found for prefix: {0} and: storage key {1}")]
    MissingValue(Prefix, StorageKey),
}

/// Error type for `StateVec` get method.
#[derive(Debug, Error)]
pub enum Error {
    /// Operation failed because the index was out of bounds.
    #[error("Index out of bounds for index: {0}")]
    IndexOutOfBounds(usize),
    /// Value not found.
    #[error("Value not found for prefix: {0} and index: {1}")]
    MissingValue(Prefix, usize),
}

// StateReaderAndWriter
pub trait StateValueAccessor<V, Codec, W>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateValueCodec<V>,
    W: StateReaderAndWriter,
{
    /// Returns the prefix used when this [`StateValue`] was created.
    fn prefix(&self) -> &Prefix;

    fn codec(&self) -> &Codec;

    /// Sets a value in the StateValue.
    fn set(&self, value: &V, working_set: &mut W) {
        working_set.set_singleton(self.prefix(), value, self.codec())
    }

    /// Gets a value from the StateValue or None if the value is absent.
    fn get(&self, working_set: &mut W) -> Option<V> {
        working_set.get_singleton(self.prefix(), self.codec())
    }

    /// Gets a value from the StateValue or Error if the value is absent.
    fn get_or_err(&self, working_set: &mut W) -> Result<V, StateValueError> {
        self.get(working_set)
            .ok_or_else(|| StateValueError::MissingValue(self.prefix().clone()))
    }

    /// Removes a value from the StateValue, returning the value (or None if the key is absent).
    fn remove(&self, working_set: &mut W) -> Option<V> {
        working_set.remove_singleton(self.prefix(), self.codec())
    }

    /// Removes a value and from the StateValue, returning the value (or Error if the key is absent).
    fn remove_or_err(&self, working_set: &mut W) -> Result<V, StateValueError> {
        self.remove(working_set)
            .ok_or_else(|| StateValueError::MissingValue(self.prefix().clone()))
    }

    /// Deletes a value from the StateValue.
    fn delete(&self, working_set: &mut W) {
        working_set.delete_singleton(self.prefix());
    }
}

pub trait StateMapAccessor<K, V, Codec, W>
where
    Codec: StateCodec,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
    W: StateReaderAndWriter,
{
    /// Returns a reference to the codec used by this [`StateMap`].
    fn codec(&self) -> &Codec;

    /// Returns the prefix used when this [`StateMap`] was created.
    fn prefix(&self) -> &Prefix;

    /// Inserts a key-value pair into the map.
    ///
    /// Much like [`StateMap::get`], the key may be any borrowed form of the
    /// map’s key type.
    fn set<Q>(&self, key: &Q, value: &V, working_set: &mut W)
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        working_set.set_value(self.prefix(), key, value, self.codec())
    }

    /// Returns the value corresponding to the key, or [`None`] if the map
    /// doesn't contain the key.
    ///
    /// # Examples
    ///
    /// The key may be any item that implements [`EncodeKeyLike`] the map's key type
    /// using your chosen codec.
    ///
    /// ```
    /// use sov_modules_api::{Context, StateMap, WorkingSet};
    ///
    /// fn foo(map: StateMap<Vec<u8>, u64>, key: &[u8], ws: &mut W) -> Option<u64>
    /// where
    ///     ,
    /// {
    ///     // We perform the `get` with a slice, and not the `Vec`. it is so because `Vec` borrows
    ///     // `[T]`.
    ///     map.get(key, ws)
    /// }
    /// ```
    ///
    /// If the map's key type does not implement [`EncodeKeyLike`] for your desired
    /// target type, you'll have to convert the key to something else. An
    /// example of this would be "slicing" an array to use in [`Vec`]-keyed
    /// maps:
    ///
    /// ```
    /// use sov_modules_api::{Context, StateMap, WorkingSet};
    ///
    /// fn foo(map: StateMap<Vec<u8>, u64>, key: [u8; 32], ws: &mut W) -> Option<u64>
    /// where
    ///     ,
    /// {
    ///     map.get(&key[..], ws)
    /// }
    /// ```
    fn get<Q>(&self, key: &Q, working_set: &mut W) -> Option<V>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        working_set.get_value(self.prefix(), key, self.codec())
    }

    /// Returns the value corresponding to the key or [`StateMapError`] if key is absent in
    /// the map.
    fn get_or_err<Q>(&self, key: &Q, working_set: &mut W) -> Result<V, StateMapError>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        self.get(key, working_set).ok_or_else(|| {
            StateMapError::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, self.codec().key_codec()),
            )
        })
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`None`] if the key is absent).
    fn remove<Q>(&self, key: &Q, working_set: &mut W) -> Option<V>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        working_set.remove_value(self.prefix(), key, self.codec())
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`StateMapError`] if the key is absent).
    ///
    /// Use [`StateMap::remove`] if you want an [`Option`] instead of a [`Result`].
    fn remove_or_err<Q>(&self, key: &Q, working_set: &mut W) -> Result<V, StateMapError>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        self.remove(key, working_set).ok_or_else(|| {
            StateMapError::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, self.codec().key_codec()),
            )
        })
    }

    /// Deletes a key-value pair from the map.
    ///
    /// This is equivalent to [`StateMap::remove`], but doesn't deserialize and
    /// return the value before deletion.
    fn delete<Q>(&self, key: &Q, working_set: &mut W)
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        working_set.delete_value(self.prefix(), key, self.codec());
    }
}

pub trait StateVecPrivateAccessor<V, Codec, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    W: StateReaderAndWriter,
{
    type ElemsMap: StateMapAccessor<usize, V, Codec, W>;
    type LenValue: StateValueAccessor<usize, Codec, W>;
    fn set_len(&self, length: usize, working_set: &mut W);

    fn elems(&self) -> &Self::ElemsMap;

    fn len_value(&self) -> &Self::LenValue;
}

pub trait StateVecAccessor<V, Codec, W>: StateVecPrivateAccessor<V, Codec, W> + Sized
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    W: StateReaderAndWriter,
{
    /// Returns the prefix used when this [`StateVec`] was created.
    fn prefix(&self) -> &Prefix;

    /// Sets a value in the [`StateVec`].
    /// If the index is out of bounds, returns an error.
    /// To push a value to the end of the StateVec, use [`StateVec::push`].
    fn set(&self, index: usize, value: &V, working_set: &mut W) -> Result<(), Error> {
        let len = self.len(working_set);

        if index < len {
            self.elems().set(&index, value, working_set);
            Ok(())
        } else {
            Err(Error::IndexOutOfBounds(index))
        }
    }

    /// Returns the value for the given index.
    fn get(&self, index: usize, working_set: &mut W) -> Option<V> {
        self.elems().get(&index, working_set)
    }

    /// Returns the value for the given index.
    /// If the index is out of bounds, returns an error.
    /// If the value is absent, returns an error.
    fn get_or_err(&self, index: usize, working_set: &mut W) -> Result<V, Error> {
        let len = self.len(working_set);

        if index < len {
            self.elems()
                .get(&index, working_set)
                .ok_or_else(|| Error::MissingValue(self.prefix().clone(), index))
        } else {
            Err(Error::IndexOutOfBounds(index))
        }
    }

    /// Returns the length of the [`StateVec`].
    fn len(&self, working_set: &mut W) -> usize {
        self.len_value().get(working_set).unwrap_or_default()
    }

    /// Pushes a value to the end of the [`StateVec`].
    fn push(&self, value: &V, working_set: &mut W) {
        let len = self.len(working_set);

        self.elems().set(&len, value, working_set);
        self.set_len(len + 1, working_set);
    }

    /// Pops a value from the end of the [`StateVec`] and returns it.
    fn pop(&self, working_set: &mut W) -> Option<V> {
        let len = self.len(working_set);
        let last_i = len.checked_sub(1)?;
        let elem = self.elems().remove(&last_i, working_set)?;

        let new_len = last_i;
        self.set_len(new_len, working_set);

        Some(elem)
    }

    /// Removes all values from this [`StateVec`].
    fn clear(&self, working_set: &mut W) {
        let len = self.len_value().remove(working_set).unwrap_or_default();

        for i in 0..len {
            self.elems().delete(&i, working_set);
        }
    }

    /// Sets all values in the [`StateVec`].
    ///
    /// If the length of the provided values is less than the length of the
    /// [`StateVec`], the remaining values will be removed from storage.
    fn set_all(&self, values: Vec<V>, working_set: &mut W) {
        let old_len = self.len(working_set);
        let new_len = values.len();

        for i in new_len..old_len {
            self.elems().delete(&i, working_set);
        }

        for (i, value) in values.into_iter().enumerate() {
            self.elems().set(&i, &value, working_set);
        }

        self.set_len(new_len, working_set);
    }

    /// Returns an iterator over all the values in the [`StateVec`].
    fn iter<'a, 'ws>(
        &'a self,
        working_set: &'ws mut W,
    ) -> StateVecIter<'a, 'ws, V, Codec, Self, W> {
        let len = self.len(working_set);
        StateVecIter {
            state_vec: self,
            ws: working_set,
            len,
            next_i: 0,
            _phantom: Default::default(),
        }
    }

    /// Returns the last value in the [`StateVec`], or [`None`] if
    /// empty.
    fn last(&self, working_set: &mut W) -> Option<V> {
        let len = self.len(working_set);
        let i = len.checked_sub(1)?;
        self.elems().get(&i, working_set)
    }
}

/// An [`Iterator`] over a [`StateVec`].
///
/// See [`StateVec::iter`] for more details.
pub struct StateVecIter<'a, 'ws, V, Codec, Vec, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    Vec: StateVecAccessor<V, Codec, W>,
    W: StateReaderAndWriter,
{
    state_vec: &'a Vec,
    ws: &'ws mut W,
    len: usize,
    next_i: usize,
    _phantom: std::marker::PhantomData<(V, Codec)>,
}

impl<'a, 'ws, V, Codec, Vec, W> Iterator for StateVecIter<'a, 'ws, V, Codec, Vec, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    Vec: StateVecAccessor<V, Codec, W>,
    W: StateReaderAndWriter,
{
    type Item = V;

    fn next(&mut self) -> Option<Self::Item> {
        let elem = self.state_vec.get(self.next_i, self.ws);
        if elem.is_some() {
            self.next_i += 1;
        }

        elem
    }
}

impl<'a, 'ws, V, Codec, Vec, W> ExactSizeIterator for StateVecIter<'a, 'ws, V, Codec, Vec, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    Vec: StateVecAccessor<V, Codec, W>,
    W: StateReaderAndWriter,
{
    fn len(&self) -> usize {
        self.len - self.next_i
    }
}

impl<'a, 'ws, V, Codec, Vec, W> FusedIterator for StateVecIter<'a, 'ws, V, Codec, Vec, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    Vec: StateVecAccessor<V, Codec, W>,
    W: StateReaderAndWriter,
{
}

impl<'a, 'ws, V, Codec, Vec, W> DoubleEndedIterator for StateVecIter<'a, 'ws, V, Codec, Vec, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    Vec: StateVecAccessor<V, Codec, W>,
    W: StateReaderAndWriter,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        self.len -= 1;
        self.state_vec.get(self.len, self.ws)
    }
}

use std::iter::FusedIterator;
use std::marker::PhantomData;

use crate::codec::{BorshCodec, StateCodec, StateKeyCodec, StateValueCodec};
use crate::{
    AccessoryStateMap, AccessoryStateValue, AccessoryWorkingSet, Prefix, StateVecError, Storage,
};

#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct AccessoryStateVec<V, Codec = BorshCodec> {
    _phantom: PhantomData<V>,
    prefix: Prefix,
    len_value: AccessoryStateValue<usize, Codec>,
    elems: AccessoryStateMap<usize, V, Codec>,
}

impl<V> AccessoryStateVec<V>
where
    BorshCodec: StateCodec + Clone,
    <BorshCodec as StateCodec>::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    <BorshCodec as StateCodec>::KeyCodec: StateKeyCodec<usize>,
{
    /// Crates a new [`AccessoryStateVec`] with the given prefix and the default
    /// [`StateValueCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self::with_codec(prefix, BorshCodec)
    }
}

impl<V, Codec> AccessoryStateVec<V, Codec>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
{
    /// Creates a new [`AccessoryStateVec`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        // Differentiating the prefixes for the length and the elements
        // shouldn't be necessary, but it's best not to rely on implementation
        // details of `StateValue` and `StateMap` as they both have the right to
        // reserve the whole key space for themselves.
        let len_value =
            AccessoryStateValue::<usize, Codec>::with_codec(prefix.extended(b"l"), codec.clone());
        let elems = AccessoryStateMap::with_codec(prefix.extended(b"e"), codec);
        Self {
            _phantom: PhantomData,
            prefix,
            len_value,
            elems,
        }
    }

    /// Returns the prefix used when this [`AccessoryStateVec`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    fn set_len<S: Storage>(&self, length: usize, working_set: &mut AccessoryWorkingSet<S>) {
        self.len_value.set(&length, working_set);
    }

    /// Sets a value in the [`AccessoryStateVec`].
    /// If the index is out of bounds, returns an error.
    /// To push a value to the end of the AccessoryStateVec, use [`AccessoryStateVec::push`].
    pub fn set<S: Storage>(
        &self,
        index: usize,
        value: &V,
        working_set: &mut AccessoryWorkingSet<S>,
    ) -> Result<(), StateVecError> {
        let len = self.len(working_set);

        if index < len {
            self.elems.set(&index, value, working_set);
            Ok(())
        } else {
            Err(StateVecError::IndexOutOfBounds(index))
        }
    }

    /// Returns the value for the given index.
    pub fn get<S: Storage>(
        &self,
        index: usize,
        working_set: &mut AccessoryWorkingSet<S>,
    ) -> Option<V> {
        self.elems.get(&index, working_set)
    }

    /// Returns the value for the given index.
    /// If the index is out of bounds, returns an error.
    /// If the value is absent, returns an error.
    pub fn get_or_err<S: Storage>(
        &self,
        index: usize,
        working_set: &mut AccessoryWorkingSet<S>,
    ) -> Result<V, StateVecError> {
        let len = self.len(working_set);

        if index < len {
            self.elems
                .get(&index, working_set)
                .ok_or_else(|| StateVecError::MissingValue(self.prefix().clone(), index))
        } else {
            Err(StateVecError::IndexOutOfBounds(index))
        }
    }

    /// Returns the length of the [`AccessoryStateVec`].
    pub fn len<S: Storage>(&self, working_set: &mut AccessoryWorkingSet<S>) -> usize {
        self.len_value.get(working_set).unwrap_or_default()
    }

    /// Pushes a value to the end of the [`AccessoryStateVec`].
    pub fn push<S: Storage>(&self, value: &V, working_set: &mut AccessoryWorkingSet<S>) {
        let len = self.len(working_set);

        self.elems.set(&len, value, working_set);
        self.set_len(len + 1, working_set);
    }

    /// Pops a value from the end of the [`AccessoryStateVec`] and returns it.
    pub fn pop<S: Storage>(&self, working_set: &mut AccessoryWorkingSet<S>) -> Option<V> {
        let len = self.len(working_set);
        let last_i = len.checked_sub(1)?;
        let elem = self.elems.remove(&last_i, working_set)?;

        let new_len = last_i;
        self.set_len(new_len, working_set);

        Some(elem)
    }

    pub fn clear<S: Storage>(&self, working_set: &mut AccessoryWorkingSet<S>) {
        let len = self.len_value.remove(working_set).unwrap_or_default();

        for i in 0..len {
            self.elems.delete(&i, working_set);
        }
    }

    /// Sets all values in the [`AccessoryStateVec`].
    ///
    /// If the length of the provided values is less than the length of the
    /// [`AccessoryStateVec`], the remaining values will be removed from storage.
    pub fn set_all<S: Storage>(&self, values: Vec<V>, working_set: &mut AccessoryWorkingSet<S>) {
        let old_len = self.len(working_set);
        let new_len = values.len();

        for i in new_len..old_len {
            self.elems.delete(&i, working_set);
        }

        for (i, value) in values.into_iter().enumerate() {
            self.elems.set(&i, &value, working_set);
        }

        self.set_len(new_len, working_set);
    }

    /// Returns an iterator over all the values in the [`AccessoryStateVec`].
    pub fn iter<'a, 'ws, S: Storage>(
        &'a self,
        working_set: &'ws mut AccessoryWorkingSet<'ws, S>,
    ) -> AccessoryStateVecIter<'a, 'ws, V, Codec, S> {
        let len = self.len(working_set);
        AccessoryStateVecIter {
            state_vec: self,
            ws: working_set,
            len,
            next_i: 0,
        }
    }

    pub fn last<S: Storage>(&self, working_set: &mut AccessoryWorkingSet<S>) -> Option<V> {
        let len = self.len(working_set);

        if len == 0usize {
            None
        } else {
            self.elems.get(&(len - 1), working_set)
        }
    }
}

/// An [`Iterator`] over a [`AccessoryStateVec`]
///
/// See [`AccessoryStateVec::iter`] for more details.
pub struct AccessoryStateVecIter<'a, 'ws, V, Codec, S>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: Storage,
{
    state_vec: &'a AccessoryStateVec<V, Codec>,
    ws: &'ws mut AccessoryWorkingSet<'ws, S>,
    len: usize,
    next_i: usize,
}

impl<'a, 'ws, V, Codec, S> Iterator for AccessoryStateVecIter<'a, 'ws, V, Codec, S>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: Storage,
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

impl<'a, 'ws, V, Codec, S> ExactSizeIterator for AccessoryStateVecIter<'a, 'ws, V, Codec, S>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: Storage,
{
    fn len(&self) -> usize {
        self.len - self.next_i
    }
}

impl<'a, 'ws, V, Codec, S> FusedIterator for AccessoryStateVecIter<'a, 'ws, V, Codec, S>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: Storage,
{
}

impl<'a, 'ws, V, Codec, S> DoubleEndedIterator for AccessoryStateVecIter<'a, 'ws, V, Codec, S>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: Storage,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        self.len -= 1;
        self.state_vec.get(self.len, self.ws)
    }
}

#[cfg(all(test, feature = "native"))]
mod test {
    use std::fmt::Debug;

    use super::*;
    use crate::{DefaultStorageSpec, ProverStorage, WorkingSet};

    enum TestCaseAction<T> {
        Push(T),
        Pop(T),
        Last(T),
        Set(usize, T),
        SetAll(Vec<T>),
        CheckLen(usize),
        CheckContents(Vec<T>),
        CheckContentsReverse(Vec<T>),
        CheckGet(usize, Option<T>),
        Clear,
    }

    fn test_cases() -> Vec<TestCaseAction<u32>> {
        vec![
            TestCaseAction::Push(1),
            TestCaseAction::Push(2),
            TestCaseAction::CheckContents(vec![1, 2]),
            TestCaseAction::CheckLen(2),
            TestCaseAction::Pop(2),
            TestCaseAction::Set(0, 10),
            TestCaseAction::CheckContents(vec![10]),
            TestCaseAction::Push(8),
            TestCaseAction::CheckContents(vec![10, 8]),
            TestCaseAction::SetAll(vec![10]),
            TestCaseAction::CheckContents(vec![10]),
            TestCaseAction::CheckGet(1, None),
            TestCaseAction::Set(0, u32::MAX),
            TestCaseAction::Push(8),
            TestCaseAction::Push(0),
            TestCaseAction::CheckContents(vec![u32::MAX, 8, 0]),
            TestCaseAction::SetAll(vec![11, 12]),
            TestCaseAction::CheckContents(vec![11, 12]),
            TestCaseAction::SetAll(vec![]),
            TestCaseAction::CheckLen(0),
            TestCaseAction::Push(42),
            TestCaseAction::Push(1337),
            TestCaseAction::Clear,
            TestCaseAction::CheckContents(vec![]),
            TestCaseAction::CheckGet(0, None),
            TestCaseAction::SetAll(vec![1, 2, 3]),
            TestCaseAction::CheckContents(vec![1, 2, 3]),
            TestCaseAction::CheckContentsReverse(vec![3, 2, 1]),
            TestCaseAction::Last(3),
        ]
    }

    #[test]
    fn test_state_vec() {
        let tmpdir = tempfile::tempdir().unwrap();
        let storage = ProverStorage::<DefaultStorageSpec>::with_path(tmpdir.path()).unwrap();
        let mut working_set = WorkingSet::new(storage);

        let prefix = Prefix::new("test".as_bytes().to_vec());
        let state_vec = AccessoryStateVec::<u32>::new(prefix);

        for test_case_action in test_cases() {
            check_test_case_action(
                &state_vec,
                test_case_action,
                &mut working_set.accessory_state(),
            );
        }
    }

    fn check_test_case_action<'ws, T, S>(
        state_vec: &AccessoryStateVec<T>,
        action: TestCaseAction<T>,
        ws: &'ws mut AccessoryWorkingSet<'ws, S>,
    ) where
        S: Storage,
        BorshCodec: StateValueCodec<T> + StateValueCodec<usize>,
        T: Eq + Debug,
    {
        match action {
            TestCaseAction::CheckContents(expected) => {
                let contents: Vec<T> = state_vec.iter(ws).collect();
                assert_eq!(expected, contents);
            }
            TestCaseAction::CheckLen(expected) => {
                let actual = state_vec.len(ws);
                assert_eq!(actual, expected);
            }
            TestCaseAction::Pop(expected) => {
                let actual = state_vec.pop(ws);
                assert_eq!(actual, Some(expected));
            }
            TestCaseAction::Push(value) => {
                state_vec.push(&value, ws);
            }
            TestCaseAction::Set(index, value) => {
                state_vec.set(index, &value, ws).unwrap();
            }
            TestCaseAction::SetAll(values) => {
                state_vec.set_all(values, ws);
            }
            TestCaseAction::CheckGet(index, expected) => {
                let actual = state_vec.get(index, ws);
                assert_eq!(actual, expected);
            }
            TestCaseAction::Clear => {
                state_vec.clear(ws);
            }
            TestCaseAction::Last(expected) => {
                let actual = state_vec.last(ws);
                assert_eq!(actual, Some(expected));
            }
            TestCaseAction::CheckContentsReverse(expected) => {
                let contents: Vec<T> = state_vec.iter(ws).rev().collect();
                assert_eq!(expected, contents);
            }
        }
    }
}

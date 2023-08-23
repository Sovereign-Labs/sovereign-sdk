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
    #[error("Index out of bounds for index: {0}")]
    IndexOutOfBounds(usize),
    #[error("Value not found for prefix: {0} and index: {1}")]
    MissingValue(Prefix, usize),
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

    fn set_len<S: Storage>(&self, length: usize, working_set: &mut WorkingSet<S>) {
        working_set.set_value(self.prefix(), &self.internal_codec(), &IndexKey(0), &length);
    }

    /// Sets a value in the [`StateVec`].
    /// If the index is out of bounds, returns an error.
    /// To push a value to the end of the StateVec, use [`StateVec::push`].
    pub fn set<S: Storage>(
        &self,
        index: usize,
        value: &V,
        working_set: &mut WorkingSet<S>,
    ) -> Result<(), Error> {
        let len = self.len(working_set);

        if index < len {
            working_set.set_value(
                self.prefix(),
                &self.internal_codec(),
                &IndexKey(index + 1),
                value,
            );
            Ok(())
        } else {
            Err(Error::IndexOutOfBounds(index))
        }
    }

    /// Returns the value for the given index.
    pub fn get<S: Storage>(&self, index: usize, working_set: &mut WorkingSet<S>) -> Option<V> {
        working_set.get_value(self.prefix(), &self.internal_codec(), &IndexKey(index + 1))
    }

    /// Returns the value for the given index.
    /// If the index is out of bounds, returns an error.
    /// If the value is absent, returns an error.
    pub fn get_or_err<S: Storage>(
        &self,
        index: usize,
        working_set: &mut WorkingSet<S>,
    ) -> Result<Option<V>, Error> {
        let len = self.len(working_set);

        if index < len {
            let elem =
                working_set.get_value(self.prefix(), &self.internal_codec(), &IndexKey(index + 1));

            if elem.is_some() {
                Ok(elem)
            } else {
                Err(Error::MissingValue(self.prefix().clone(), index))
            }
        } else {
            Err(Error::IndexOutOfBounds(index))
        }
    }

    /// Returns the length of the [`StateVec`].
    pub fn len<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> usize {
        let len = working_set.get_value::<_, usize, _>(
            self.prefix(),
            &self.internal_codec(),
            &IndexKey(0),
        );
        len.unwrap_or_default()
    }

    /// Pushes a value to the end of the [`StateVec`].
    pub fn push<S: Storage>(&self, value: &V, working_set: &mut WorkingSet<S>) {
        let len = self.len(working_set);

        working_set.set_value(
            self.prefix(),
            &self.internal_codec(),
            &IndexKey(len + 1),
            value,
        );
        self.set_len(len + 1, working_set);
    }

    /// Pops a value from the end of the [`StateVec`] and returns it.
    pub fn pop<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
        let len = self.len(working_set);

        if len > 0 {
            let elem =
                working_set.remove_value(self.prefix(), &self.internal_codec(), &IndexKey(len));
            self.set_len(len - 1, working_set);
            elem
        } else {
            None
        }
    }

    pub fn clear<S: Storage>(&self, working_set: &mut WorkingSet<S>) {
        let len = self.len(working_set);

        for _ in 0..len {
            working_set.delete_value(self.prefix(), &self.internal_codec(), &IndexKey(len));
        }
        self.set_len(0, working_set);
    }

    /// Sets all values in the [`StateVec`].
    /// If the length of the provided values is less than the length of the [`StateVec`], the remaining values stay in storage but are inaccessible.
    pub fn set_all<S: Storage>(&self, values: Vec<V>, working_set: &mut WorkingSet<S>) {
        // TODO(performance): optimize this, we could skip many reads and writes here.
        self.clear(working_set);

        for value in values.into_iter() {
            self.push(&value, working_set);
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
            Ok(IndexKey(usize::from_be_bytes(
                bytes.try_into().expect("Couldn't cast to [u8; 8]"),
            )))
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

#[cfg(test)]
mod test {
    use std::fmt::Debug;

    use super::*;
    use crate::{DefaultStorageSpec, ProverStorage};

    enum TestCaseAction<T> {
        Push(T),
        Pop(T),
        Set(usize, T),
        SetAll(Vec<T>),
        CheckLen(usize),
        CheckContents(Vec<T>),
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
            TestCaseAction::Set(0, u32::MAX),
            TestCaseAction::Push(0),
            TestCaseAction::CheckContents(vec![u32::MAX, 8, 0]),
            TestCaseAction::SetAll(vec![11, 12]),
            TestCaseAction::CheckContents(vec![11, 12]),
            TestCaseAction::SetAll(vec![]),
            TestCaseAction::CheckLen(0),
            TestCaseAction::Push(0),
            TestCaseAction::Clear,
            TestCaseAction::CheckContents(vec![]),
        ]
    }

    fn get_all<T, VC, C>(sv: &StateVec<T, VC>, ws: &mut WorkingSet<C>) -> Vec<T>
    where
        VC: StateValueCodec<T> + StateValueCodec<usize>,
        C: Storage,
    {
        let mut result = Vec::new();
        let len = sv.len(ws);
        for i in 0..len {
            result.push(sv.get(i, ws).unwrap());
        }
        result
    }

    fn check_test_case_action<T, S>(
        state_vec: &StateVec<T>,
        action: TestCaseAction<T>,
        ws: &mut WorkingSet<S>,
    ) where
        S: Storage,
        BorshCodec: StateValueCodec<T> + StateValueCodec<usize>,
        T: Eq + Debug,
    {
        match action {
            TestCaseAction::CheckContents(expected) => {
                assert_eq!(expected, get_all(&state_vec, ws));
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
            TestCaseAction::Clear => {
                state_vec.clear(ws);
            }
        }
    }

    #[test]
    fn test_state_vec() {
        let tmpdir = tempfile::tempdir().unwrap();
        let storage = ProverStorage::<DefaultStorageSpec>::with_path(tmpdir.path()).unwrap();
        let mut working_set = WorkingSet::new(storage);

        let prefix = Prefix::new("test".as_bytes().to_vec());
        let state_vec = StateVec::<u32>::new(prefix.clone());

        for test_case_action in test_cases() {
            check_test_case_action(&state_vec, test_case_action, &mut working_set);
        }
    }
}

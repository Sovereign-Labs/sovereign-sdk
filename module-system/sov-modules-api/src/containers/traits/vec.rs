use std::iter::FusedIterator;

use sov_modules_core::{Prefix, StateCodec, StateKeyCodec, StateReaderAndWriter, StateValueCodec};
use thiserror::Error;

use crate::{StateMapAccessor, StateValueAccessor};

/// An error type for vector getters.
#[derive(Debug, Error)]
pub enum StateVecError {
    /// Operation failed because the index was out of bounds.
    #[error("Index out of bounds for index: {0}")]
    IndexOutOfBounds(usize),
    /// Value not found.
    #[error("Value not found for prefix: {0} and index: {1}")]
    MissingValue(Prefix, usize),
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
    /// Returns the prefix used when this vector was created.
    fn prefix(&self) -> &Prefix;

    /// Sets a value in the vector.
    /// If the index is out of bounds, returns an error.
    /// To push a value to the end of the StateVec, use [`StateVecAccessor::push`].
    fn set(&self, index: usize, value: &V, working_set: &mut W) -> Result<(), StateVecError> {
        let len = self.len(working_set);

        if index < len {
            self.elems().set(&index, value, working_set);
            Ok(())
        } else {
            Err(StateVecError::IndexOutOfBounds(index))
        }
    }

    /// Returns the value for the given index.
    fn get(&self, index: usize, working_set: &mut W) -> Option<V> {
        self.elems().get(&index, working_set)
    }

    /// Returns the value for the given index.
    /// If the index is out of bounds, returns an error.
    /// If the value is absent, returns an error.
    fn get_or_err(&self, index: usize, working_set: &mut W) -> Result<V, StateVecError> {
        let len = self.len(working_set);

        if index < len {
            self.elems()
                .get(&index, working_set)
                .ok_or_else(|| StateVecError::MissingValue(self.prefix().clone(), index))
        } else {
            Err(StateVecError::IndexOutOfBounds(index))
        }
    }

    /// Returns the length of the vector.
    fn len(&self, working_set: &mut W) -> usize {
        self.len_value().get(working_set).unwrap_or_default()
    }

    /// Pushes a value to the end of the vector.
    fn push(&self, value: &V, working_set: &mut W) {
        let len = self.len(working_set);

        self.elems().set(&len, value, working_set);
        self.set_len(len + 1, working_set);
    }

    /// Pops a value from the end of the vector and returns it.
    fn pop(&self, working_set: &mut W) -> Option<V> {
        let len = self.len(working_set);
        let last_i = len.checked_sub(1)?;
        let elem = self.elems().remove(&last_i, working_set)?;

        let new_len = last_i;
        self.set_len(new_len, working_set);

        Some(elem)
    }

    /// Removes all values from this vector.
    fn clear(&self, working_set: &mut W) {
        let len = self.len_value().remove(working_set).unwrap_or_default();

        for i in 0..len {
            self.elems().delete(&i, working_set);
        }
    }

    /// Sets all values in the tector.
    ///
    /// If the length of the provided values is less than the length of the
    /// vector, the remaining values will be removed from storage.
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

    /// Returns an iterator over all the values in the vector.
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

    /// Returns the last value in the vector, or [`None`] if
    /// empty.
    fn last(&self, working_set: &mut W) -> Option<V> {
        let len = self.len(working_set);
        let i = len.checked_sub(1)?;
        self.elems().get(&i, working_set)
    }
}

/// An [`Iterator`] over a state vector.
///
/// See [`StateVecAccessor::iter`] for more details.
pub struct StateVecIter<'a, 'ws, V, Codec, S, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: StateVecAccessor<V, Codec, W>,
    W: StateReaderAndWriter,
{
    state_vec: &'a S,
    ws: &'ws mut W,
    len: usize,
    next_i: usize,
    _phantom: std::marker::PhantomData<(V, Codec)>,
}

impl<'a, 'ws, V, Codec, S, W> Iterator for StateVecIter<'a, 'ws, V, Codec, S, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: StateVecAccessor<V, Codec, W>,
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

impl<'a, 'ws, V, Codec, S, W> ExactSizeIterator for StateVecIter<'a, 'ws, V, Codec, S, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: StateVecAccessor<V, Codec, W>,
    W: StateReaderAndWriter,
{
    fn len(&self) -> usize {
        self.len - self.next_i
    }
}

impl<'a, 'ws, V, Codec, S, W> FusedIterator for StateVecIter<'a, 'ws, V, Codec, S, W>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    S: StateVecAccessor<V, Codec, W>,
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

#[cfg(test)]
pub mod tests {
    use std::fmt::Debug;

    use sov_state::codec::BorshCodec;

    use super::*;

    pub trait Testable<W>: StateVecAccessor<u32, BorshCodec, W>
    where
        W: StateReaderAndWriter,
    {
        fn run_tests(&self, working_set: &mut W) {
            for test_case_action in test_cases() {
                check_test_case_action(self, test_case_action, working_set);
            }
        }
    }

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

    impl<T: StateVecAccessor<u32, BorshCodec, W>, W> Testable<W> for T where W: StateReaderAndWriter {}

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

    fn check_test_case_action<S, T, W>(state_vec: &S, action: TestCaseAction<T>, ws: &mut W)
    where
        S: StateVecAccessor<T, BorshCodec, W>,
        BorshCodec: StateValueCodec<T>,
        T: Eq + Debug,
        W: StateReaderAndWriter,
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

use std::marker::PhantomData;

use sov_modules_core::{
    AccessoryWorkingSet, Context, Prefix, StateCodec, StateKeyCodec, StateValueCodec,
};
use sov_state::codec::BorshCodec;

use crate::{StateValueAccessor, StateVecAccessor};

use super::{traits::StateVecPrivateAccessor, AccessoryStateMap, AccessoryStateValue};

/// A variant of [`StateVec`](crate::StateVec) that stores its elements as
/// "accessory" state, instead of in the JMT.
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

impl<'a, V, Codec, C> StateVecPrivateAccessor<V, Codec, AccessoryWorkingSet<'a, C>>
    for AccessoryStateVec<V, Codec>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    C: Context,
{
    type ElemsMap = AccessoryStateMap<usize, V, Codec>;

    type LenValue = AccessoryStateValue<usize, Codec>;

    fn set_len(&self, length: usize, working_set: &mut AccessoryWorkingSet<'a, C>) {
        self.len_value.set(&length, working_set);
    }

    fn elems(&self) -> &Self::ElemsMap {
        &self.elems
    }

    fn len_value(&self) -> &Self::LenValue {
        &self.len_value
    }
}

impl<'a, V, Codec, C> StateVecAccessor<V, Codec, AccessoryWorkingSet<'a, C>>
    for AccessoryStateVec<V, Codec>
where
    Codec: StateCodec + Clone,
    Codec::ValueCodec: StateValueCodec<V> + StateValueCodec<usize>,
    Codec::KeyCodec: StateKeyCodec<usize>,
    C: Context,
{
    /// Returns the prefix used when this [`StateVec`] was created.
    fn prefix(&self) -> &Prefix {
        &self.prefix
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
}

#[cfg(all(test, feature = "native"))]
mod test {
    use std::fmt::Debug;

    use sov_modules_core::WorkingSet;
    use sov_state::{DefaultStorageSpec, ProverStorage};

    use super::*;
    use crate::default_context::DefaultContext;

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
        let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(storage);

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

    fn check_test_case_action<'ws, T, C>(
        state_vec: &AccessoryStateVec<T>,
        action: TestCaseAction<T>,
        ws: &'ws mut AccessoryWorkingSet<'ws, C>,
    ) where
        BorshCodec: StateValueCodec<T> + StateValueCodec<usize>,
        T: Eq + Debug,
        C: Context,
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

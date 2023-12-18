use std::marker::PhantomData;

use sov_modules_core::{
    AccessoryWorkingSet, Context, Prefix, StateCodec, StateKeyCodec, StateValueCodec,
};
use sov_state::codec::BorshCodec;

use super::traits::StateVecPrivateAccessor;
use super::{AccessoryStateMap, AccessoryStateValue};
use crate::{StateValueAccessor, StateVecAccessor};

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
    /// Returns the prefix used when this vector was created.
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

    use sov_modules_core::WorkingSet;
    use sov_prover_storage_manager::new_orphan_storage;

    use super::*;
    use crate::containers::traits::vec_tests::Testable;
    use crate::default_context::DefaultContext;

    #[test]
    fn test_accessory_state_vec() {
        let tmpdir = tempfile::tempdir().unwrap();
        let storage = new_orphan_storage(tmpdir.path()).unwrap();
        let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(storage);

        let prefix = Prefix::new("test".as_bytes().to_vec());
        let state_vec = AccessoryStateVec::<u32>::new(prefix);
        state_vec.run_tests(&mut working_set.accessory_state())
    }
}

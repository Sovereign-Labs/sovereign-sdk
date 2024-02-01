use std::marker::PhantomData;

use sov_modules_core::{
    AccessoryWorkingSet, Context, Prefix, StateCodec, StateKeyCodec, StateValueCodec,
};
use sov_state::codec::BorshCodec;

use super::traits::StateMapAccessor;

/// A container that maps keys to values stored as "accessory" state, outside of
/// the JMT.
///
/// # Type parameters
/// [`AccessoryStateMap`] is generic over:
/// - a key type `K`;
/// - a value type `V`;
/// - a [`StateValueCodec`] `Codec`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct AccessoryStateMap<K, V, Codec = BorshCodec> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    codec: Codec,
    prefix: Prefix,
}

impl<K, V> AccessoryStateMap<K, V> {
    /// Creates a new [`AccessoryStateMap`] with the given prefix and the default
    /// [`StateValueCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self::with_codec(prefix, BorshCodec)
    }
}

impl<K, V, Codec> AccessoryStateMap<K, V, Codec> {
    /// Creates a new [`AccessoryStateMap`] with the given prefix and [`StateValueCodec`].
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`AccessoryStateMap`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

impl<'a, K, V, Codec, C> StateMapAccessor<K, V, Codec, AccessoryWorkingSet<'a, C>>
    for AccessoryStateMap<K, V, Codec>
where
    Codec: StateCodec,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
    C: Context,
{
    /// Returns the prefix used when this [`AccessoryStateMap`] was created.
    fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    fn codec(&self) -> &Codec {
        &self.codec
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, K, V, Codec> AccessoryStateMap<K, V, Codec>
where
    K: arbitrary::Arbitrary<'a>,
    V: arbitrary::Arbitrary<'a>,
    Codec: StateCodec + Default,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
{
    /// Generates an arbitrary [`AccessoryStateMap`] instance.
    ///
    /// See the [`arbitrary`] crate for more information.
    pub fn arbitrary_working_set<C>(
        u: &mut arbitrary::Unstructured<'a>,
        working_set: &mut AccessoryWorkingSet<C>,
    ) -> arbitrary::Result<Self>
    where
        C: Context,
    {
        use arbitrary::Arbitrary;

        let prefix = Prefix::arbitrary(u)?;
        let len = u.arbitrary_len::<(K, V)>()?;
        let codec = Codec::default();
        let map = Self::with_codec(prefix, codec);

        (0..len).try_fold(map, |map, _| {
            let key = K::arbitrary(u)?;
            let value = V::arbitrary(u)?;

            map.set(&key, &value, working_set);

            Ok(map)
        })
    }
}

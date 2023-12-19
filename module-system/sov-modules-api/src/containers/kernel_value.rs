use std::marker::PhantomData;

use sov_modules_core::{Context, KernelWorkingSet, Prefix, StateCodec, StateValueCodec};
use sov_state::codec::BorshCodec;

use super::traits::StateValueAccessor;

/// Container for a single value which is only accesible in the kernel.
#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct KernelStateValue<V, Codec = BorshCodec> {
    _phantom: PhantomData<V>,
    codec: Codec,
    prefix: Prefix,
}

impl<V> KernelStateValue<V> {
    /// Crates a new [`KernelStateValue`] with the given prefix and the default
    /// [`StateValueCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self::with_codec(prefix, BorshCodec)
    }
}

impl<V, Codec> KernelStateValue<V, Codec> {
    /// Creates a new [`KernelStateValue`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            _phantom: PhantomData,
            codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`KernelStateValue`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

impl<'a, V, Codec, C> StateValueAccessor<V, Codec, KernelWorkingSet<'a, C>>
    for KernelStateValue<V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateValueCodec<V>,
    C: Context,
{
    fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    fn codec(&self) -> &Codec {
        &self.codec
    }
}

use std::convert::Infallible;

use sov_state::{
    CompileTimeNamespace, SlotKey, SlotValue, StateCodec, StateItemCodec, StateItemDecoder,
};

use super::UniversalStateAccessor;
use crate::{StateReader, StateWriter};

/// A wrapper around an accessor that does not charge gas for state accesses.
/// This is used in the testing framework to wrap the [`crate::WorkingSet`] and avoid charging gas in the `post_dispatch_hook` checks for tests.
/// It is also currently used in the `EVM` module to avoid double-charging gas for state accesses.
pub struct UnmeteredStateWrapper<'a, T> {
    pub(crate) inner: &'a mut T,
}

impl<T: UniversalStateAccessor> UniversalStateAccessor for UnmeteredStateWrapper<'_, T> {
    fn get_size(&mut self, namespace: sov_state::Namespace, key: &SlotKey) -> Option<u32> {
        self.inner.get_size(namespace, key)
    }

    fn get_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey) -> Option<SlotValue> {
        self.inner.get_value(namespace, key)
    }

    fn set_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey, value: SlotValue) {
        self.inner.set_value(namespace, key, value);
    }

    fn delete_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey) {
        self.inner.delete_value(namespace, key);
    }
}

impl<Inner> UnmeteredStateWrapper<'_, Inner> {
    /// Returns a reference to the inner state accessor.
    pub fn inner(&self) -> &Inner {
        self.inner
    }

    /// Returns a mutable reference to the inner state accessor. Use this with caution, since operations on the inner accessor may be metered.
    pub fn inner_mut(&mut self) -> &mut Inner {
        self.inner
    }
}

impl<Inner, N: CompileTimeNamespace> StateReader<N> for UnmeteredStateWrapper<'_, Inner>
where
    Inner: StateReader<N>,
{
    type Error = Infallible;

    fn get(&mut self, key: &SlotKey) -> Result<Option<SlotValue>, Self::Error> {
        Ok(self.inner.get_value(N::NAMESPACE, key))
    }

    fn get_decoded<V, Codec>(
        &mut self,
        storage_key: &SlotKey,
        codec: &Codec,
    ) -> Result<Option<V>, Self::Error>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateItemCodec<V>,
    {
        let storage_value = <Self as StateReader<N>>::get(self, storage_key)?;

        Ok(storage_value
            .map(|storage_value| codec.value_codec().decode_unwrap(storage_value.value())))
    }
}

impl<Inner, N: CompileTimeNamespace> StateWriter<N> for UnmeteredStateWrapper<'_, Inner>
where
    Inner: StateWriter<N>,
{
    type Error = Infallible;

    fn set(&mut self, key: &SlotKey, value: SlotValue) -> Result<(), Self::Error> {
        self.inner.set_value(N::NAMESPACE, key, value);
        Ok(())
    }

    fn delete(&mut self, key: &SlotKey) -> Result<(), Self::Error> {
        self.inner.delete_value(N::NAMESPACE, key);
        Ok(())
    }
}

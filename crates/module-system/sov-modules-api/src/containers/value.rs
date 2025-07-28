use std::marker::PhantomData;

use sov_state::codec::BorshCodec;
use sov_state::namespaces::{Accessory, CompileTimeNamespace, Kernel, User};
use sov_state::{EncodeLike, Prefix, SlotKey, SlotValue, StateCodec, StateItemCodec};
use thiserror::Error;

use super::{Borrowed, BorrowedMut};
use crate::{StateReader, StateReaderAndWriter, StateWriter};

/// Container for a single value.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct NamespacedStateValue<N, V, Codec = BorshCodec>
where
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
{
    _phantom: PhantomData<(V, N)>,
    pub(crate) codec: Codec,
    pub(crate) prefix: Prefix,
}

/// Error type for getters from state values method.
#[derive(Debug, Error)]
pub enum StateValueError<N: CompileTimeNamespace> {
    /// The value was not found for the combination of (namespace, prefix) provided.
    #[error("Value not found for prefix: {0} in namespace: {}", std::any::type_name::<N>())]
    MissingValue(Prefix, PhantomData<N>),
}

type ValueOrError<V, N> = Result<V, StateValueError<N>>;

/// A container for a single user-space value.
pub type StateValue<V, Codec = BorshCodec> = NamespacedStateValue<User, V, Codec>;
/// A Container for a single value which is only accesible in the kernel.
pub type KernelStateValue<V, Codec = BorshCodec> = NamespacedStateValue<Kernel, V, Codec>;
/// A Container for a single value stored as "accessory" state, outside of the
/// JMT.
pub type AccessoryStateValue<V, Codec = BorshCodec> = NamespacedStateValue<Accessory, V, Codec>;

// Implement all other functions generically over codecs
impl<N, V, Codec> NamespacedStateValue<N, V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    N: CompileTimeNamespace,
{
    /// Creates a new [`StateValue`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            _phantom: PhantomData,
            codec,
            prefix,
        }
    }

    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    /// Returns the codec used for this value
    pub fn codec(&self) -> &Codec {
        &self.codec
    }

    /// Returns the `SlotKey` for this value
    pub fn slot_key(&self) -> SlotKey {
        SlotKey::singleton(self.prefix())
    }

    /// Encodes the provided value into a slot value
    pub(crate) fn slot_value(&self, value: &V) -> SlotValue
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateItemCodec<V>,
    {
        SlotValue::new(value, self.codec().value_codec())
    }

    /// Sets the value.
    pub fn set<Vq, Writer>(&mut self, value: &V, state: &mut Writer) -> Result<(), Writer::Error>
    where
        Vq: ?Sized,
        Codec::ValueCodec: EncodeLike<Vq, V>,
        Writer: StateWriter<N>,
    {
        let key = self.slot_key();
        tracing::trace!(%key, "Setting state value");
        state.set(&key, self.slot_value(value))
    }

    /// Gets the value from state or returns None if the value is absent.
    pub fn get<Reader: StateReader<N>>(
        &self,
        state: &mut Reader,
    ) -> Result<Option<V>, Reader::Error> {
        let key = self.slot_key();
        tracing::trace!(%key, "Getting state value");
        state.get_decoded(&key, self.codec())
    }

    /// Returns a borrowed value from the map, preventing mutable access to the map until this reference is dropped.
    pub fn borrow<Reader>(
        &self,
        state: &mut Reader,
    ) -> Result<Borrowed<Option<V>, Self>, Reader::Error>
    where
        Reader: StateReader<N>,
    {
        let key = self.slot_key();
        tracing::trace!(%key, "Borrowing state value");
        Ok(Borrowed::new(state.get_decoded(&key, self.codec())?, self))
    }

    /// Returns a mutably borrowed value from the map, preventing access to the map until this reference is dropped.
    pub fn borrow_mut<Reader>(
        &mut self,
        state: &mut Reader,
    ) -> Result<BorrowedMut<Option<V>, Self>, Reader::Error>
    where
        Reader: StateReader<N>,
    {
        let key = self.slot_key();
        tracing::trace!(%key, "Borrowing mut state value");
        let val = state.get_decoded(&key, self.codec())?;
        Ok(BorrowedMut::new(key, val, self))
    }

    /// Gets the value from state or Error if the value is absent.
    pub fn get_or_err<Reader: StateReader<N>>(
        &self,
        state: &mut Reader,
    ) -> Result<ValueOrError<V, N>, Reader::Error> {
        Ok(self
            .get(state)?
            .ok_or_else(|| StateValueError::<N>::MissingValue(self.prefix().clone(), PhantomData)))
    }

    /// Removes the value from state, returning the value (or None if the key is absent).
    pub fn remove<ReaderAndWriter: StateReaderAndWriter<N>>(
        &mut self,
        state: &mut ReaderAndWriter,
    ) -> Result<Option<V>, <ReaderAndWriter as StateWriter<N>>::Error> {
        let key = self.slot_key();
        tracing::trace!(%key, "Removing state value");
        state.remove_decoded(&key, self.codec())
    }

    /// Removes a value from state, returning the value (or Error if the key is absent).
    pub fn remove_or_err<ReaderAndWriter: StateReaderAndWriter<N>>(
        &mut self,
        state: &mut ReaderAndWriter,
    ) -> Result<ValueOrError<V, N>, <ReaderAndWriter as StateWriter<N>>::Error> {
        Ok(self
            .remove(state)?
            .ok_or_else(|| StateValueError::<N>::MissingValue(self.prefix().clone(), PhantomData)))
    }

    /// Deletes a value from state.
    pub fn delete<Writer: StateWriter<N>>(
        &mut self,
        state: &mut Writer,
    ) -> Result<(), Writer::Error> {
        let key = self.slot_key();
        tracing::trace!(%key, "Deleting state value");
        state.delete(&key)
    }
}

#[cfg(feature = "native")]
mod proofs {
    use sov_state::namespaces::ProvableCompileTimeNamespace;
    use sov_state::{StateCodec, StateItemCodec, StateItemDecoder, Storage};

    use super::NamespacedStateValue;
    use crate::{ProvenStateAccessor, Spec};

    impl<N, V, Codec> NamespacedStateValue<N, V, Codec>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateItemCodec<V>,
        N: ProvableCompileTimeNamespace,
    {
        /// Gets the value with a proof of correctness.
        pub fn get_with_proof<W>(&self, state: &mut W) -> Option<sov_state::StorageProof<W::Proof>>
        where
            W: ProvenStateAccessor<N>,
        {
            state.get_with_proof(self.slot_key())
        }

        pub fn verify_proof<S: Spec>(
            &self,
            state_root: <S::Storage as Storage>::Root,
            proof: sov_state::StorageProof<<<S as Spec>::Storage as Storage>::Proof>,
        ) -> anyhow::Result<Option<V>>
where {
            anyhow::ensure!(
                proof.namespace == N::PROVABLE_NAMESPACE,
                "The provided proof comes from a different namespace. Expected {:?} but found {:?}",
                N::PROVABLE_NAMESPACE,
                proof.namespace
            );

            let (key, value) = S::Storage::open_proof(state_root, proof)?;
            anyhow::ensure!(
                key == self.slot_key(),
                "The provided proof is for a different key. Expected {:?} but found {:?}",
                self.slot_key(),
                key
            );

            value
                .map(|v| {
                    self.codec()
                        .value_codec()
                        .try_decode(v.value())
                        .map_err(|e| anyhow::anyhow!("Failed to decode value from proof: {:?}", e))
                })
                .transpose()
        }
    }
}

use std::convert::Infallible;
use std::fmt::Debug;
use std::num::TryFromIntError;

use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
#[cfg(feature = "native")]
use sov_state::StorageProof;
use sov_state::{
    namespaces, Accessory, CompileTimeNamespace, EventContainer, Kernel, Namespace,
    ProvableCompileTimeNamespace, SlotKey, SlotValue, StateCodec, StateItemCodec, StateItemDecoder,
    User,
};
use thiserror::Error;
use tracing::{enabled, instrument, Level, Span};

use super::accessors::seal::UniversalStateAccessor;
use super::accessors::{BorshSerializedSize, TempCache};
use crate::capabilities::RollupHeight;
#[cfg(any(feature = "test-utils", feature = "evm"))]
use crate::UnmeteredStateWrapper;
use crate::{Gas, GasMeter, GasMeteringError, GasSpec, RevertableTxState, Spec};

/// A type that can both read and write the normal "user-space" state of the rollup.
///
/// ```
/// fn delete_state_string<Accessor: sov_modules_api::StateAccessor>(mut value: sov_modules_api::StateValue<String>, state: &mut Accessor)
///  -> Result<(), <Accessor as sov_modules_api::StateWriter<sov_state::User>>::Error> {
///     if let Some(original) = value.get(state)? {
///         println!("original: {}", original);
///     }
///     value.delete(state)?;
///     Ok(())
/// }
///
///
/// ```
pub trait StateAccessor: StateReaderAndWriter<User> {
    /// Converts this accessor into an [`UnmeteredStateWrapper`]. This method should only be used either in tests or in the `EVM` module.
    #[cfg(any(feature = "test-utils", feature = "evm"))]
    fn to_unmetered(&mut self) -> UnmeteredStateWrapper<Self>
    where
        Self: Sized,
    {
        UnmeteredStateWrapper { inner: self }
    }
}

/// A trait that represents a [`StateAccessor`] that never fails on state accesses. Accessing the state with structs that implement
/// this trait will return [`Infallible`].
///
/// ## Usage example
/// ```
/// use sov_modules_api::prelude::UnwrapInfallible;
///
/// fn delete_state_string<InfallibleAccessor: sov_modules_api::InfallibleStateAccessor>(mut value: sov_modules_api::StateValue<String>, state: &mut InfallibleAccessor)
///  -> () {
///     if let Some(original) = value.get(state).unwrap_infallible() {
///         println!("original: {}", original);
///     }
///     value.delete(state).unwrap_infallible();
/// }
/// ```
pub trait InfallibleStateAccessor:
    StateReader<User, Error = Infallible> + StateWriter<User, Error = Infallible>
{
}

impl<T> StateAccessor for T where T: StateReaderAndWriter<User> {}

impl<T> InfallibleStateAccessor for T where
    T: StateReader<User, Error = Infallible> + StateWriter<User, Error = Infallible>
{
}

/// Like [`InfallibleStateAccessor`], but for the [`Kernel`] access.
pub trait InfallibleKernelStateAccessor:
    StateReader<Kernel, Error = Infallible> + StateWriter<Kernel, Error = Infallible>
{
}

impl<T> InfallibleKernelStateAccessor for T where
    T: StateReader<Kernel, Error = Infallible> + StateWriter<Kernel, Error = Infallible>
{
}

/// The state accessor used during transaction execution. It provides unrestricted
/// access to [`User`]-space state, as well as limited visibility into the `Kernel` state.
pub trait TxState<S: Spec>:
    StateReader<User, Error: Into<anyhow::Error>>
    + StateReader<Kernel, Error = <Self as StateReader<User>>::Error>
    + StateWriter<User, Error = <Self as StateReader<User>>::Error>
    + StateWriter<Kernel, Error = <Self as StateReader<User>>::Error>
    + StateWriter<Accessory, Error = Infallible>
    + VersionReader
    + EventContainer
    + PerBlockCache
    + GasMeter<Spec = S>
    + Sized
{
    /// Converts this state accessor into a [`RevertableTxState`].
    ///
    /// You *MUST* call .commit() to save the changes from the resulting accessor if you want them to be persisted
    fn to_revertable(&mut self) -> RevertableTxState<S, Self> {
        RevertableTxState::new(self)
    }
}

impl<S: Spec, T> TxState<S> for T where
    T: StateReader<User, Error: Into<anyhow::Error>>
        + StateReader<Kernel, Error = <Self as StateReader<User>>::Error>
        + StateWriter<Kernel, Error = <Self as StateReader<User>>::Error>
        + StateWriter<User, Error = <Self as StateReader<User>>::Error>
        + StateWriter<Accessory, Error = Infallible>
        + VersionReader
        + EventContainer
        + PerBlockCache
        + GasMeter<Spec = S>
        + Sized
{
}

/// A cache that persists items *without serializing them*. Items persist for at most the duration of the block.
///
/// Note that values may be evicted from the cache at any time based on memory pressure, even if the end of the block has not yet been reached.
pub trait PerBlockCache {
    /// Gets a value from the cache. This API returns &T because mutating the type would invalidate the revert
    /// guarantees provided by the SDK. Be extremely careful when using interior mutability for objects stored in the cache -
    /// any changes made to the object may not revert on transaction failure, causing possible cache corruption.
    fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T>;
    /// Puts a value in the cache. Note that values are required to provide an esimate of their size via the
    /// [`BorshSerializedSize`] trait.
    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T);
    /// Deletes a value from the cache.
    fn delete_cached<T: 'static + Send + Sync>(&mut self);
    /// Adds all writes from another cache to this one.
    fn update_cache_with(&mut self, other: TempCache);
}

/// The state accessor used during genesis. It provides unrestricted
/// access to [`User`] and `Kernel` state, as well as limited visibility into [`Accessory`] state.  
pub trait GenesisState<S: Spec>:
    TxState<S> + PrivilegedKernelAccessor<Error = <Self as StateReader<User>>::Error>
{
}

/// The set of errors that can be raised during state accesses. For now all these errors are
/// caused by gas metering issues, hence this error type is a wrapper around the [`GasMeteringError`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StateAccessorError<GU: Gas> {
    /// An error occurred when trying to get a value from the state.
    #[error(
        "An error occured while trying to get the value (key {key:}) from the state: {inner}, namespace: {namespace:?}"
    )]
    Get {
        /// The key of the value that was not found.
        key: SlotKey,
        /// The error that occurred while trying to get the value.
        inner: GasMeteringError<GU>,
        /// The namespace that was queried.
        namespace: Namespace,
    },
    /// An error occurred when trying to set a value in the state.
    #[error(
        "An error occurred while trying to set the value (key {key}) in the state: {inner}, namespace: {namespace:?}"
    )]
    Set {
        /// The key of the value that was not found.
        key: SlotKey,
        /// The error that occurred while trying to set the value.
        inner: GasMeteringError<GU>,
        /// The namespace that was queried.
        namespace: Namespace,
    },
    /// An error occurred when trying to decode a value retrieved from the state.
    #[error(
        "An error occured while trying to decode the value (key {key:}) in the state: {inner}, namespace: {namespace:?}"
    )]
    Decode {
        /// The key of the value that was not found.
        key: SlotKey,
        /// The error that occurred while trying to decode the value.
        inner: GasMeteringError<GU>,
        /// The namespace that was queried.
        namespace: Namespace,
    },
    /// An error occurred when trying to delete a value from the state.
    #[error(
        "An error occured while trying to delete the value (key {key:}) in the state: {inner}, namespace: {namespace:?}"
    )]
    Delete {
        /// The key of the value that was not found.
        key: SlotKey,
        /// The error that occurred while trying to delete the value.
        inner: GasMeteringError<GU>,
        /// The namespace that was queried.
        namespace: Namespace,
    },
}

/// A trait that represents a [`StateReader`] and [`StateWriter`] to a given namespace that never fails on state accesses. Accessing the state with structs that implement
/// this trait will return [`Infallible`].
///
/// ## Usage example
/// ```
/// use sov_modules_api::prelude::UnwrapInfallible;
/// use sov_state::namespaces::User;
///
/// fn delete_state_string<InfallibleAccessor: sov_modules_api::InfallibleStateReaderAndWriter<User>>
/// (mut value: sov_modules_api::StateValue<String>, state: &mut InfallibleAccessor) -> () {
///     if let Some(original) = value.get(state).unwrap_infallible() {
///         println!("original: {}", original);
///     }
///     value.delete(state).unwrap_infallible();
/// }
/// ```
pub trait InfallibleStateReaderAndWriter<N: CompileTimeNamespace>:
    StateReader<N, Error = Infallible> + StateWriter<N, Error = Infallible>
{
}

impl<
        T: StateReader<N, Error = Infallible> + StateWriter<N, Error = Infallible>,
        N: CompileTimeNamespace,
    > InfallibleStateReaderAndWriter<N> for T
{
}

/// A trait that represents a [`StateReader`] and [`StateWriter`] to the accessory namespace that never fails on state accesses.
/// Basically a [`InfallibleStateReaderAndWriter<Accessory>`] for the accessory namespace.
pub trait AccessoryStateReaderAndWriter: InfallibleStateReaderAndWriter<Accessory> {}
impl<T: InfallibleStateReaderAndWriter<Accessory>> AccessoryStateReaderAndWriter for T {}

/// A wrapper trait for storage reader and writer that can be used to charge gas
/// for the read/write operations.
pub trait StateReaderAndWriter<N: CompileTimeNamespace>:
    StateReader<N> + StateWriter<N, Error = <Self as StateReader<N>>::Error>
{
    /// Removes a value from storage and decode the result
    fn remove_decoded<V, Codec>(
        &mut self,
        key: &SlotKey,
        codec: &Codec,
    ) -> Result<Option<V>, <Self as StateWriter<N>>::Error>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateItemCodec<V>,
    {
        let value = self.get_decoded(key, codec)?;
        <Self as StateWriter<N>>::delete(self, key)?;
        Ok(value)
    }
}

impl<T, N> StateReaderAndWriter<N> for T
where
    T: StateReader<N> + StateWriter<N, Error = <Self as StateReader<N>>::Error>,
    N: CompileTimeNamespace,
{
}

/// A storage reader which can access a particular namespace.
pub trait StateReader<N: CompileTimeNamespace>: UniversalStateAccessor {
    /// The error type returned when a state read operation fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Get a value from the storage. Basically a wrapper around [`StateReader::get`].
    ///
    /// ## Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the read operation.
    fn get(&mut self, key: &SlotKey) -> Result<Option<SlotValue>, Self::Error>;

    /// Get a decoded value from the storage.
    ///
    /// ## Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the read operation.
    ///
    /// ## Note
    /// For now this method doesn't charge an extra amount of gas for the decoding operation. This may change in the future.
    fn get_decoded<V, Codec>(
        &mut self,
        storage_key: &SlotKey,
        codec: &Codec,
    ) -> Result<Option<V>, Self::Error>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateItemCodec<V>;
}

/// A storage reader which can access the accessory state.
/// Does not charge gas for read operations.
pub trait AccessoryStateReader: UniversalStateAccessor {}

/// A trait wrapper that replicates the functionality of [`StateReader`] but with a gas metering interface.
/// This allows a storage reader to charge gas for read operations.
pub trait ProvableStateReader<N: ProvableCompileTimeNamespace>:
    UniversalStateAccessor + GasMeter
{
}

macro_rules! blanket_impl_metered_state_reader {
    ($namespace:ty) => {
        type Error = StateAccessorError<<T::Spec as GasSpec>::Gas>;

        fn get(&mut self, key: &SlotKey) -> Result<Option<SlotValue>, Self::Error> {
            let val = get_inner(
                self,
                <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                key,
            )
            .map_err(|e| StateAccessorError::Get {
                key: key.clone(),
                inner: e,
                namespace: <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
            })?;

            Ok(val)
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
            let storage_value = <Self as StateReader<$namespace>>::get(self, storage_key)?;

            storage_value
                .map(|storage_value| {
                    // We need to charge for the cost to deserialize the value
                    tracing::trace_span!("all_accesses::charge_per_byte_borsh_deserialization",)
                        .in_scope(|| {
                            self.charge_linear_gas(
                                &<T::Spec as GasSpec>::gas_to_charge_per_byte_borsh_deserialization(
                                ),
                                storage_value.size(),
                            )
                        })
                        .map_err(|e| StateAccessorError::Decode {
                            key: storage_key.clone(),
                            inner: e,
                            namespace: <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                        })?;

                    Ok(codec.value_codec().decode_unwrap(storage_value.value()))
                })
                .transpose()
        }
    };
}

impl<T: ProvableStateReader<Kernel>> StateReader<Kernel> for T {
    blanket_impl_metered_state_reader!(Kernel);
}

impl<T: ProvableStateReader<User>> StateReader<User> for T {
    blanket_impl_metered_state_reader!(User);
}

impl<T: AccessoryStateReader> StateReader<Accessory> for T {
    type Error = Infallible;

    /// Get a value from the storage.
    fn get(&mut self, key: &SlotKey) -> Result<Option<SlotValue>, Self::Error> {
        Ok(self.get_value(Accessory::NAMESPACE, key))
    }

    /// Get a decoded value from the storage.
    fn get_decoded<V, Codec>(
        &mut self,
        storage_key: &SlotKey,
        codec: &Codec,
    ) -> Result<Option<V>, Self::Error>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateItemCodec<V>,
    {
        let storage_value = <Self as StateReader<Accessory>>::get(self, storage_key)?;

        Ok(storage_value
            .map(|storage_value| codec.value_codec().decode_unwrap(storage_value.value())))
    }
}

/// Provides write-only access to a particular namespace
pub trait StateWriter<N: CompileTimeNamespace>: UniversalStateAccessor {
    /// The error type returned when a state write operation fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Sets a value in the storage. Basically a wrapper around [`StateWriter::set`].
    ///
    /// ## Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the write operation.
    fn set(&mut self, key: &SlotKey, value: SlotValue) -> Result<(), Self::Error>;

    /// Deletes a value from the storage. Basically a wrapper around [`StateWriter::delete`].
    ///
    /// ## Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the delete operation.
    fn delete(&mut self, key: &SlotKey) -> Result<(), Self::Error>;
}

/// A trait wrapper that replicates the functionality of [`StateWriter`] but with a gas metering interface.
/// This allows a storage writer to charge gas for write operations.
pub trait ProvableStateWriter<N: ProvableCompileTimeNamespace>:
    UniversalStateAccessor + GasMeter
{
}

macro_rules! blanket_impl_metered_state_writer {
    ($namespace:ty) => {
        impl<T: ProvableStateWriter<$namespace>> StateWriter<$namespace> for T {
            type Error = StateAccessorError<<T::Spec as GasSpec>::Gas>;

            fn set(&mut self, key: &SlotKey, value: SlotValue) -> Result<(), Self::Error> {
                set_inner(
                    self,
                    <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                    key,
                    value,
                )
                .map_err(|e| StateAccessorError::Set {
                    key: key.clone(),
                    inner: e,
                    namespace: <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                })?;

                Ok(())
            }

            fn delete(&mut self, key: &SlotKey) -> Result<(), Self::Error> {
                delete_inner(
                    self,
                    <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                    key,
                )
                .map_err(|e| StateAccessorError::Delete {
                    key: key.clone(),
                    inner: e,
                    namespace: <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                })?;
                Ok(())
            }
        }
    };
}

blanket_impl_metered_state_writer!(User);
blanket_impl_metered_state_writer!(Kernel);

/// Provides write-only access to the accessory state
/// Does not charge gas for write/delete operations.
pub trait AccessoryStateWriter: UniversalStateAccessor {}

impl<T: AccessoryStateWriter> StateWriter<Accessory> for T {
    type Error = Infallible;

    /// Replaces a storage value.
    fn set(&mut self, key: &SlotKey, value: SlotValue) -> Result<(), Self::Error> {
        self.set_value(Accessory::NAMESPACE, key, value);
        Ok(())
    }

    /// Deletes a storage value.
    fn delete(&mut self, key: &SlotKey) -> Result<(), Self::Error> {
        self.delete_value(Accessory::NAMESPACE, key);
        Ok(())
    }
}

#[cfg(feature = "native")]
/// Allows a type to retrieve state values with a proof of their presence/absence.
pub trait ProvenStateAccessor<N: ProvableCompileTimeNamespace>: StateReaderAndWriter<N> {
    /// The underlying storage whose proof is returned
    type Proof;
    /// Fetch the value with the requested key and provide a proof of its presence/absence.
    fn get_with_proof(&mut self, key: SlotKey) -> Option<StorageProof<Self::Proof>>
    where
        Self: StateReaderAndWriter<N>,
        N: ProvableCompileTimeNamespace;
}

/// A [`StateReader`] that is version-aware.
pub trait VersionReader {
    /// Returns the largest slot number that the accessor is allowed to access. During transaction execution,
    /// this is the same as the value returned by [`VersionReader::current_visible_slot_number`]. When executing with kernel,
    /// permissions, this is the true slot number. Note: Kernel permissions are only applicable to maintainers of the SDK.
    fn max_allowed_slot_number_to_access(&self) -> SlotNumber;

    /// Returns the current visible slot number.
    fn current_visible_slot_number(&self) -> VisibleSlotNumber;

    /// Returns the current version of the state accessor
    fn rollup_height_to_access(&self) -> RollupHeight;
}

/// A trait for state accessors that can know the true [`SlotNumber`] and use it to read/write the kernel.
/// Note that this trait should be implemented with extreme care, since misuse can cause accidental breakage of
/// soft confirmations. In particular, this trait should never be added to [`TxState`].
pub trait PrivilegedKernelAccessor: StateWriter<namespaces::Kernel> {
    /// Returns the current true rollup height contained in the accessor
    fn true_slot_number(&self) -> SlotNumber;
}

/// Amount to pay for access to a storage value.
fn charge_storage_access<Accessor: UniversalStateAccessor + GasMeter>(
    accessor: &mut Accessor,
    key: &SlotKey,
) -> Result<(), GasMeteringError<<Accessor::Spec as Spec>::Gas>> {
    // Charge:
    // - cold access bias to load something from the storage (aka Merkle proof cost)
    // - fixed hashing cost
    // - hashing cost of the key length
    tracing::trace_span!("access::charge_bias_for_access",).in_scope(|| {
        accessor.charge_gas(&<Accessor::Spec as GasSpec>::bias_to_charge_for_access())
    })?;

    tracing::trace_span!("access::charge_hash_update",).in_scope(|| {
        accessor.charge_gas(&<Accessor::Spec as GasSpec>::gas_to_charge_hash_update())
    })?;

    let key_size: u32 = key
        .size()
        .try_into()
        .map_err(|e: TryFromIntError| GasMeteringError::Overflow(e.to_string()))?;

    if key_size > 0 {
        tracing::trace_span!("access::charge_per_byte_hash_update").in_scope(|| {
            accessor.charge_linear_gas(
                &<Accessor::Spec as GasSpec>::gas_to_charge_per_byte_hash_update(),
                key_size,
            )
        })?;
    }

    Ok(())
}

fn charge_read<Accessor: UniversalStateAccessor + GasMeter>(
    accessor: &mut Accessor,
    namespace: Namespace,
    key: &SlotKey,
) -> Result<(), GasMeteringError<<Accessor::Spec as Spec>::Gas>> {
    charge_storage_access(accessor, key)?;

    tracing::trace_span!("access::charge_bias_for_read",).in_scope(|| {
        accessor.charge_gas(&<Accessor::Spec as GasSpec>::bias_to_charge_for_read())
    })?;

    let value_size = accessor.get_size(namespace, key);

    match value_size {
        Some(0) | None => {}
        Some(value_size) => {
            tracing::trace_span!("access::charge_per_byte_read").in_scope(|| {
                accessor.charge_linear_gas(
                    &<Accessor::Spec as GasSpec>::gas_to_charge_per_byte_read(),
                    value_size,
                )
            })?;

            tracing::trace_span!("access::charge_hash_update", value_size = value_size).in_scope(
                || accessor.charge_gas(&<Accessor::Spec as GasSpec>::gas_to_charge_hash_update()),
            )?;

            tracing::trace_span!(
                "access::charge_per_byte_hash_update",
                value_size = value_size
            )
            .in_scope(|| {
                accessor.charge_linear_gas(
                    &<Accessor::Spec as GasSpec>::gas_to_charge_per_byte_hash_update(),
                    value_size,
                )
            })?;
        }
    }

    Ok(())
}

fn charge_write<Accessor: UniversalStateAccessor + GasMeter>(
    accessor: &mut Accessor,
    _namespace: Namespace,
    key: &SlotKey,
    value_size: u32,
) -> Result<(), GasMeteringError<<Accessor::Spec as Spec>::Gas>> {
    charge_storage_access(accessor, key)?;

    accessor.charge_gas(&<Accessor::Spec as GasSpec>::bias_to_charge_storage_update())?;
    accessor.charge_linear_gas(
        &<Accessor::Spec as GasSpec>::gas_to_charge_per_byte_storage_update(),
        value_size,
    )?;

    accessor.charge_gas(&<Accessor::Spec as GasSpec>::gas_to_charge_hash_update())?;

    accessor.charge_linear_gas(
        &<Accessor::Spec as GasSpec>::gas_to_charge_per_byte_hash_update(),
        value_size,
    )?;

    Ok(())
}

#[instrument(name = "state::get", skip_all, level = Level::TRACE, fields(key = %key, value_size_bytes))]
pub(crate) fn get_inner<Accessor: UniversalStateAccessor + GasMeter>(
    accessor: &mut Accessor,
    namespace: Namespace,
    key: &SlotKey,
) -> Result<Option<SlotValue>, GasMeteringError<<Accessor::Spec as Spec>::Gas>> {
    charge_read(accessor, namespace, key)?;

    let value = accessor.get_value(namespace, key);

    if enabled!(Level::TRACE) {
        let size = value.as_ref().map(SlotValue::size).unwrap_or(0);
        Span::current().record("value_size_bytes", size);
    }

    Ok(value)
}

#[instrument(name = "state::set", skip_all, level = Level::TRACE, fields(key = %key, value_size_bytes = value.size()))]
pub(crate) fn set_inner<Accessor: UniversalStateAccessor + GasMeter>(
    accessor: &mut Accessor,
    namespace: Namespace,
    key: &SlotKey,
    value: SlotValue,
) -> Result<(), GasMeteringError<<Accessor::Spec as Spec>::Gas>> {
    charge_write(accessor, namespace, key, value.size())?;

    accessor.set_value(namespace, key, value);

    Ok(())
}

#[instrument(name = "state::delete", skip_all, level = Level::TRACE, fields(key = %key, value_size_bytes))]
pub(crate) fn delete_inner<Accessor: UniversalStateAccessor + GasMeter>(
    accessor: &mut Accessor,
    namespace: Namespace,
    key: &SlotKey,
) -> Result<(), GasMeteringError<<Accessor::Spec as Spec>::Gas>> {
    // Doing a delete is the same as doing a write with a size of 0
    charge_write(accessor, namespace, key, 0)?;

    // avoid an extra size calculation
    if enabled!(Level::TRACE) {
        let size = accessor.get_size(namespace, key).unwrap_or(0);
        Span::current().record("value_size_bytes", size);
    }

    accessor.delete_value(namespace, key);

    Ok(())
}

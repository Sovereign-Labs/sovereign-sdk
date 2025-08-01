//! Runtime state machine definitions.

use std::collections::HashMap;
use std::marker::PhantomData;

use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::{
    EventContainer, Kernel as KernelType, Namespace, SlotKey, SlotValue, TypeErasedEvent, User,
};

use super::checkpoints::StateCheckpoint;
use super::internals::RevertableWriter;
use super::temp_cache::{CacheLookup, TempCache};
use super::{BorshSerializedSize, ChangeSet, StateProvider, UniversalStateAccessor};
use crate::capabilities::RollupHeight;
use crate::module::Spec;
use crate::state::traits::PerBlockCache;
use crate::transaction::{
    transaction_consumption_helper, AuthenticatedTransactionData, PriorityFeeBips,
    TransactionConsumption,
};
#[cfg(feature = "test-utils")]
use crate::{AccessoryStateReader, GasArray};
use crate::{
    AccessoryStateWriter, Amount, BasicGasMeter, Gas, GasInfo, GasMeter, GasMeteringError,
    GetGasPrice, ProvableStateReader, ProvableStateWriter, TxState, VersionReader,
};

/// A state diff over the storage that contains all the changes related to transaction execution.
///
/// This structure is built from a [`StateProvider`] (typically a
/// [`StateCheckpoint`]) and is used in the entire transaction lifecycle (from
/// pre-execution checks to post execution state updates).
///
/// ## Usage note
/// This method tracks the gas consumed outside of the transaction lifecycle without explicitly consuming a finite resource.
/// This should only be used in infailible methods.
pub struct RevertableTxState<'a, S: Spec, State> {
    pub(super) inner: &'a mut State,
    pub(super) events: Vec<TypeErasedEvent>,
    pub(super) temp_cache: TempCache,
    pub(super) writes: HashMap<(Namespace, SlotKey), Option<SlotValue>>,
    pub(super) phantom: PhantomData<S>,
}

impl<'a, S: Spec, I: TxState<S>> RevertableTxState<'a, S, I> {
    /// Creates a new [`RevertableTxState`] from the provided [`TxState`].
    ///
    /// # Important
    /// You *MUST* call [`RevertableTxState::commit`] to save any changes made to this state.
    pub fn new(inner: &'a mut I) -> Self {
        Self {
            inner,
            events: Vec::default(),
            temp_cache: TempCache::new(),
            writes: HashMap::default(),
            phantom: PhantomData,
        }
    }

    /// Commits the changes from this [`RevertableTxState`] into the underlying state.
    pub fn commit(self) -> &'a mut I {
        for event in self.events {
            self.inner.add_type_erased_event(event);
        }
        for (key, value) in self.writes {
            if let Some(value) = value {
                self.inner.set_value(key.0, &key.1, value);
            } else {
                self.inner.delete_value(key.0, &key.1);
            }
        }
        self.inner.update_cache_with(self.temp_cache);
        self.inner
    }

    /// Reverts the changes from this [`RevertableTxState`] and returns the underlying state.
    pub fn revert(self) -> &'a mut I {
        self.inner
    }
}

impl<S: Spec, I: TxState<S>> PerBlockCache for RevertableTxState<'_, S, I> {
    fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T> {
        match self.temp_cache.get::<T>() {
            CacheLookup::Hit(value) => value,
            CacheLookup::Miss => self.inner.get_cached::<T>(),
        }
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T) {
        self.temp_cache.set(value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self) {
        self.temp_cache.delete::<T>();
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.temp_cache.update_with(other);
    }
}

impl<S: Spec, I: TxState<S>> VersionReader for RevertableTxState<'_, S, I> {
    fn rollup_height_to_access(&self) -> RollupHeight {
        self.inner.rollup_height_to_access()
    }

    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.inner.current_visible_slot_number()
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.inner.max_allowed_slot_number_to_access()
    }
}

impl<S: Spec, I: TxState<S>> UniversalStateAccessor for RevertableTxState<'_, S, I> {
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        if let Some(value) = self.writes.get(&(namespace, key.clone())) {
            return value.as_ref().map(|v| v.size());
        }
        self.inner.get_size(namespace, key)
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        if let Some(value) = self.writes.get(&(namespace, key.clone())) {
            return value.clone();
        }
        self.inner.get_value(namespace, key)
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.writes.insert((namespace, key.clone()), Some(value));
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        self.writes.insert((namespace, key.clone()), None);
    }
}

impl<S: Spec, I: TxState<S>> GasMeter for RevertableTxState<'_, S, I> {
    type Spec = S;
    fn charge_gas(&mut self, amount: &S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        self.inner.charge_gas(amount)
    }

    fn charge_linear_gas(
        &mut self,
        amount: &<Self::Spec as Spec>::Gas,
        parameter: u32,
    ) -> anyhow::Result<(), GasMeteringError<<Self::Spec as Spec>::Gas>> {
        self.inner.charge_linear_gas(amount, parameter)
    }

    #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
    fn remove_gas_pattern(&mut self, amount: &<Self::Spec as Spec>::Gas, parameter: u32) {
        self.inner.remove_gas_pattern(amount, parameter);
    }
}

impl<S: Spec, I: TxState<S>> ProvableStateReader<User> for RevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> ProvableStateReader<KernelType> for RevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> ProvableStateWriter<User> for RevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> ProvableStateWriter<KernelType> for RevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> AccessoryStateWriter for RevertableTxState<'_, S, I> {}
#[cfg(feature = "test-utils")]
impl<S: Spec, I: TxState<S>> AccessoryStateReader for RevertableTxState<'_, S, I> {}

impl<S: Spec, I: TxState<S>> EventContainer for RevertableTxState<'_, S, I> {
    fn add_event<E: 'static + core::marker::Send>(&mut self, event_key: &str, event: E) {
        self.events.push(TypeErasedEvent::new(event_key, event));
    }

    fn add_type_erased_event(&mut self, event: TypeErasedEvent) {
        self.events.push(event);
    }
}

/// A state diff over the storage that contains all the changes related to transaction execution.
///
/// This structure is built from a [`StateProvider`] (typically a
/// [`StateCheckpoint`]) and is used in the entire transaction lifecycle (from
/// pre-execution checks to post execution state updates).
///
/// ## Usage note
/// This method tracks the gas consumed outside of the transaction lifecycle without explicitly consuming a finite resource.
/// This should only be used in infailible methods.
pub struct TxScratchpad<S: Spec, I: StateProvider<S>> {
    pub(super) inner: RevertableWriter<I>,
    pub(super) phantom: PhantomData<S>,
}

impl<S: Spec, I: StateProvider<S>> UniversalStateAccessor for TxScratchpad<S, I> {
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        <RevertableWriter<I> as UniversalStateAccessor>::get_size(&mut self.inner, namespace, key)
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        <RevertableWriter<I> as UniversalStateAccessor>::get_value(&mut self.inner, namespace, key)
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        <RevertableWriter<I> as UniversalStateAccessor>::set_value(
            &mut self.inner,
            namespace,
            key,
            value,
        );
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        <RevertableWriter<I> as UniversalStateAccessor>::delete_value(
            &mut self.inner,
            namespace,
            key,
        );
    }
}

impl<S: Spec, I: StateProvider<S>> GasMeter for TxScratchpad<S, I> {
    type Spec = S;
}

/// The list of changes caused by a single transaction
#[derive(Debug, Clone)]
pub struct TxChangeSet(pub ChangeSet);

impl<S: Spec, I: StateProvider<S>> TxScratchpad<S, I> {
    /// Commits the changes of this [`TxScratchpad`] and returns a [`StateCheckpoint`].
    pub fn commit(self) -> I {
        self.inner.commit()
    }

    /// Gets an iterator over the diff currently written onto this scratchpad. These changes will
    /// be reverted or committed as a unit.
    pub fn tx_changes(&self) -> TxChangeSet {
        TxChangeSet(self.inner.changes())
    }

    /// Reverts the changes of this [`TxScratchpad`] and returns a [`StateCheckpoint`].
    pub fn revert(self) -> I {
        self.inner.revert()
    }

    /// Converts this [`TxScratchpad`] into a [`PreExecWorkingSet`].
    pub fn to_pre_exec_working_set(self, gas_meter: BasicGasMeter<S>) -> PreExecWorkingSet<S, I> {
        PreExecWorkingSet {
            inner: self,
            gas_meter,
        }
    }
}

impl<S: Spec, I: StateProvider<S>> VersionReader for TxScratchpad<S, I> {
    fn rollup_height_to_access(&self) -> RollupHeight {
        self.inner.inner.rollup_height_to_access()
    }

    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.inner.inner.current_visible_slot_number()
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.inner.inner.max_allowed_slot_number_to_access()
    }
}

impl<S: Spec, I: StateProvider<S>> PerBlockCache for TxScratchpad<S, I> {
    fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T> {
        self.inner.get_cached::<T>()
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T) {
        self.inner.cache_writes.set(value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self) {
        self.inner.cache_writes.delete::<T>();
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.inner.cache_writes.update_with(other);
    }
}

/// A working set that can be used to charge gas for pre transaction execution checks.
pub struct PreExecWorkingSet<S: Spec, I: StateProvider<S>> {
    inner: TxScratchpad<S, I>,
    gas_meter: BasicGasMeter<S>,
}

impl<S: Spec, I: StateProvider<S>> PreExecWorkingSet<S, I> {
    /// Returns the associated gas meter and the scratchpad.
    #[must_use]
    pub fn to_scratchpad_and_gas_meter(self) -> (TxScratchpad<S, I>, BasicGasMeter<S>) {
        (self.inner, self.gas_meter)
    }

    /// Commits the contents of the [`PreExecWorkingSet`].
    #[must_use]
    pub fn commit(self) -> Self {
        let inner = self.inner.commit();
        let scratchpad = inner.to_tx_scratchpad();
        scratchpad.to_pre_exec_working_set(self.gas_meter)
    }

    /// Reverts all changes up to the last commit.
    #[must_use]
    pub fn revert(self) -> (TxScratchpad<S, I>, BasicGasMeter<S>) {
        let inner = self.inner.revert();
        let scratchpad = inner.to_tx_scratchpad();
        (scratchpad, self.gas_meter)
    }
}

impl<S: Spec, I: StateProvider<S>> GasMeter for PreExecWorkingSet<S, I> {
    type Spec = S;
    fn charge_gas(&mut self, amount: &S::Gas) -> anyhow::Result<(), GasMeteringError<S::Gas>> {
        self.gas_meter.charge_gas(amount)
    }

    fn charge_linear_gas(
        &mut self,
        amount: &<Self::Spec as Spec>::Gas,
        parameter: u32,
    ) -> anyhow::Result<(), GasMeteringError<<Self::Spec as Spec>::Gas>> {
        self.gas_meter.charge_linear_gas(amount, parameter)
    }

    #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
    fn remove_gas_pattern(&mut self, amount: &<Self::Spec as Spec>::Gas, parameter: u32) {
        self.gas_meter.remove_gas_pattern(amount, parameter);
    }
}

impl<S: Spec, I: StateProvider<S>> GetGasPrice for PreExecWorkingSet<S, I> {
    type Spec = S;
    fn gas_price(&self) -> &<S::Gas as Gas>::Price {
        self.gas_meter.gas_price()
    }
}

impl<S: Spec, I: StateProvider<S>> UniversalStateAccessor for PreExecWorkingSet<S, I> {
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        <TxScratchpad<S, I> as UniversalStateAccessor>::get_size(&mut self.inner, namespace, key)
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        <TxScratchpad<S, I> as UniversalStateAccessor>::get_value(&mut self.inner, namespace, key)
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        <TxScratchpad<S, I> as UniversalStateAccessor>::set_value(
            &mut self.inner,
            namespace,
            key,
            value,
        );
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        <TxScratchpad<S, I> as UniversalStateAccessor>::delete_value(
            &mut self.inner,
            namespace,
            key,
        );
    }
}

impl<S: Spec, I: StateProvider<S>> VersionReader for PreExecWorkingSet<S, I> {
    fn rollup_height_to_access(&self) -> RollupHeight {
        self.inner.rollup_height_to_access()
    }

    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.inner.current_visible_slot_number()
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.inner.max_allowed_slot_number_to_access()
    }
}

#[cfg(feature = "test-utils")]
impl<S: Spec> StateCheckpoint<S> {
    /// Produces an unmetered [`WorkingSet`] from this [`StateProvider`].
    /// This is useful for tests that don't need to track gas consumption.
    pub fn to_working_set_unmetered(self) -> WorkingSet<S, Self> {
        WorkingSet {
            delta: RevertableWriter::new(self.to_tx_scratchpad()),
            events: Default::default(),
            gas_meter: BasicGasMeter::new_with_gas(
                <S::Gas as crate::Gas>::max(),
                <S::Gas as crate::Gas>::Price::ZEROED,
            ),
            max_fee: Amount::ZERO,
            max_priority_fee_bips: PriorityFeeBips::ZERO,
        }
    }
}

/// This structure contains the read-write set and the events collected during
/// the execution of a transaction.
///
/// There are two ways to convert it into a [`StateCheckpoint`]:
///
/// 1. By using the [`WorkingSet::finalize`] method, where all the changes are
///    added to the underlying [`TxScratchpad`].
/// 2. By using the [`WorkingSet::revert`] method, where the most recent changes
///    are reverted and the previous [`TxScratchpad`] is returned.
pub struct WorkingSet<S: Spec, I: StateProvider<S> = StateCheckpoint<S>> {
    pub(super) delta: RevertableWriter<TxScratchpad<S, I>>,
    events: Vec<TypeErasedEvent>,
    gas_meter: BasicGasMeter<S>,
    // Gas parameters of the transaction associated with the working set
    max_fee: Amount,
    max_priority_fee_bips: PriorityFeeBips,
}

impl<S: Spec, I: StateProvider<S>> WorkingSet<S, I> {
    /// Get the `GasInfo` for the `WorkingSet`.
    pub fn gas_info(&self) -> GasInfo<<S as Spec>::Gas> {
        self.gas_meter.gas_info()
    }

    /// Creates a new [`WorkingSet`] from the provided [`TxScratchpad`] and [`AuthenticatedTransactionData`].
    pub fn create_working_set(
        scratchpad: TxScratchpad<S, I>,
        tx: &AuthenticatedTransactionData<S>,
        working_set_gas_meter: BasicGasMeter<S>,
    ) -> Self {
        Self {
            delta: RevertableWriter::new(scratchpad),
            events: Vec::default(),
            gas_meter: working_set_gas_meter,
            max_fee: tx.0.max_fee,
            max_priority_fee_bips: tx.0.max_priority_fee_bips,
        }
    }

    /// Builds a [`crate::TransactionConsumption`] from the [`WorkingSet`].
    pub(crate) fn transaction_consumption(&self) -> TransactionConsumption<S::Gas> {
        // The base fee is the amount of gas consumed by the transaction execution.
        // The `base_fee` is retrieved from self.gas_meter, which guards against base_fee * gas_price overflow.
        let base_fee = self.gas_meter.gas_info().gas_used;
        let gas_price = self.gas_meter.gas_info().gas_price;

        transaction_consumption_helper::<S>(
            &base_fee,
            &gas_price,
            self.max_fee,
            self.max_priority_fee_bips,
        )
    }

    /// Turns this [`WorkingSet`] into a [`TxScratchpad`], commits the changes to the [`WorkingSet`] to the
    /// inner scratchpad.
    #[allow(clippy::type_complexity)]
    pub fn finalize(
        self,
    ) -> (
        TxScratchpad<S, I>,
        TransactionConsumption<S::Gas>,
        Vec<TypeErasedEvent>,
    ) {
        let tx_reward = self.transaction_consumption();
        (self.delta.commit(), tx_reward, self.events)
    }

    /// Reverts the most recent changes to this [`WorkingSet`], returning a pristine
    /// [`TxScratchpad`] instance.
    pub fn revert(self) -> (TxScratchpad<S, I>, TransactionConsumption<S::Gas>) {
        let tx_consumption = self.transaction_consumption();
        (self.delta.revert(), tx_consumption)
    }

    /// Extracts all typed events from this working set.
    pub fn take_events(&mut self) -> Vec<TypeErasedEvent> {
        core::mem::take(&mut self.events)
    }

    /// Extracts a typed event at index `index`
    pub fn take_event(&mut self, index: usize) -> Option<TypeErasedEvent> {
        if index < self.events.len() {
            Some(self.events.remove(index))
        } else {
            None
        }
    }

    /// Returns an immutable map of all typed events that have been previously
    /// written to this working set.
    pub fn events(&self) -> &[TypeErasedEvent] {
        &self.events
    }

    /// Returns the maximum fee that can be paid for this transaction expressed in gas token amount.
    pub fn max_fee(&self) -> Amount {
        self.max_fee
    }
}

#[cfg(test)]
use crate::capabilities::Kernel;

#[cfg(test)]
impl<S: Spec> WorkingSet<S, StateCheckpoint<S>> {
    /// A helper function to create a new [`WorkingSet`] with a given gas price and remaining funds.
    /// Note: This method uses a [`MockKernel`] with a default height, this is not compatible with tests over multiple slots.
    pub fn new_with_gas_meter(
        inner: S::Storage,
        remaining_funds: crate::Amount,
        price: &<S::Gas as crate::Gas>::Price,
    ) -> Self {
        use crate::capabilities::mocks::MockKernel;

        let state_checkpoint: StateCheckpoint<S> =
            StateCheckpoint::new(inner, &MockKernel::<S>::default());
        let tx_scratchpad = TxScratchpad {
            inner: RevertableWriter::new(state_checkpoint),
            phantom: PhantomData,
        };

        WorkingSet {
            delta: RevertableWriter::new(tx_scratchpad),
            events: Default::default(),
            gas_meter: BasicGasMeter::new_with_funds_and_gas(
                remaining_funds,
                <S::Gas as crate::Gas>::max(),
                price.clone(),
            ),
            max_fee: Amount::ZERO,
            max_priority_fee_bips: PriorityFeeBips::ZERO,
        }
    }

    /// Creates a new [`WorkingSet`] instance backed by the given [`Spec::Storage`] and a [`Kernel`].
    pub fn new_with_kernel<K: Kernel<S>>(inner: S::Storage, kernel: &K) -> Self {
        let state_checkpoint: StateCheckpoint<S> = StateCheckpoint::new(inner, kernel);
        let tx_scratchpad = TxScratchpad {
            inner: RevertableWriter::new(state_checkpoint),
            phantom: PhantomData,
        };

        WorkingSet {
            delta: RevertableWriter::new(tx_scratchpad),
            events: Default::default(),
            gas_meter: BasicGasMeter::new_with_gas(
                <S::Gas as crate::Gas>::max(),
                <S::Gas as crate::Gas>::Price::ZEROED,
            ),
            max_fee: Amount::ZERO,
            max_priority_fee_bips: PriorityFeeBips::ZERO,
        }
    }
}

impl<S: Spec, I: StateProvider<S>> GasMeter for WorkingSet<S, I> {
    type Spec = S;

    fn charge_gas(&mut self, gas: &S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        self.gas_meter.charge_gas(gas)
    }

    fn charge_linear_gas(
        &mut self,
        amount: &<Self::Spec as Spec>::Gas,
        parameter: u32,
    ) -> anyhow::Result<(), GasMeteringError<<Self::Spec as Spec>::Gas>> {
        self.gas_meter.charge_linear_gas(amount, parameter)
    }

    #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
    fn remove_gas_pattern(&mut self, amount: &<Self::Spec as Spec>::Gas, parameter: u32) {
        self.gas_meter.remove_gas_pattern(amount, parameter);
    }
}

impl<S: Spec, I: StateProvider<S>> GetGasPrice for WorkingSet<S, I> {
    type Spec = S;
    fn gas_price(&self) -> &<S::Gas as Gas>::Price {
        self.gas_meter.gas_price()
    }
}

impl<S: Spec, I: StateProvider<S>> UniversalStateAccessor for WorkingSet<S, I> {
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        self.delta.get_size(namespace, key)
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        self.delta.get_value(namespace, key)
    }
    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.delta.set_value(namespace, key, value);
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        self.delta.delete_value(namespace, key);
    }
}

impl<S: Spec, I: StateProvider<S>> EventContainer for WorkingSet<S, I> {
    fn add_event<E: 'static + core::marker::Send>(&mut self, event_key: &str, event: E) {
        self.events.push(TypeErasedEvent::new(event_key, event));
    }

    fn add_type_erased_event(&mut self, event: TypeErasedEvent) {
        self.events.push(event);
    }
}

impl<S: Spec, I: StateProvider<S>> VersionReader for WorkingSet<S, I> {
    fn rollup_height_to_access(&self) -> RollupHeight {
        self.delta.inner.rollup_height_to_access()
    }

    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.delta.inner.current_visible_slot_number()
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.delta.inner.max_allowed_slot_number_to_access()
    }
}

impl<S: Spec, I: StateProvider<S>> PerBlockCache for WorkingSet<S, I> {
    fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T> {
        self.delta.get_cached::<T>()
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T) {
        self.delta.cache_writes.set(value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self) {
        self.delta.cache_writes.delete::<T>();
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.delta.cache_writes.update_with(other);
    }
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::common::HexString;
    use sov_state::codec::BcsCodec;
    use sov_state::namespaces::User;
    use sov_state::{Kernel, SlotKey, SlotValue};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::{MockDaSpec, MockZkvm};

    use crate::capabilities::mocks::MockKernel;
    use crate::capabilities::Kernel as _;
    use crate::execution_mode::Native;
    use crate::{
        BasicGasMeter, GasArray, Spec, StateAccessor, StateCheckpoint, StateProvider, StateReader,
        StateWriter, WorkingSet,
    };

    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn test_workingset_get() {
        let codec = BcsCodec {};
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();

        let prefix = sov_state::Prefix::new(vec![1, 2, 3]);
        let storage_key = SlotKey::new::<HexString, _, _>(&prefix, [4, 5, 6].as_ref(), &codec);
        let storage_value = SlotValue::new(&vec![7, 8, 9], &codec);

        let mut working_set =
            WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        StateWriter::<User>::set(&mut working_set, &storage_key, storage_value.clone()).expect("The set operation should succeed because there should be enough funds in the metered working set");
        let value = StateReader::<User>::get(&mut working_set, &storage_key).expect("The get operation should succeed because there should be enough funds in the metered working set");

        assert_eq!(Some(storage_value), value);
    }

    #[test]
    fn test_kernel_workingset_get() {
        let codec = BcsCodec {};
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();

        let prefix = sov_state::Prefix::new(vec![1, 2, 3]);
        let storage_key = SlotKey::new::<HexString, _, _>(&prefix, [4, 5, 6].as_ref(), &codec);
        let storage_value = SlotValue::new(&vec![7, 8, 9], &codec);
        let kernel: MockKernel<TestSpec> = MockKernel::new(4, 1);

        let mut working_set = StateCheckpoint::<TestSpec>::new(storage.clone(), &kernel);
        let mut working_set = kernel.accessor(&mut working_set);

        StateWriter::<Kernel>::set(&mut working_set, &storage_key, storage_value.clone())
            .expect("This should be unfaillible");

        assert_eq!(
            Some(storage_value),
            StateReader::<Kernel>::get(&mut working_set, &storage_key)
                .expect("This should be unfaillible")
        );
    }

    fn save_and_check_value<ST: StateAccessor>(key: &SlotKey, val: SlotValue, accessor: &mut ST) {
        StateWriter::<User>::set(accessor, key, val.clone()).expect("This should be unfaillible");
        assert_eq!(
            Some(val),
            StateReader::<User>::get(accessor, key).expect("This should be unfaillible")
        );
    }

    #[test]
    fn test_pre_exec_ws() {
        let codec = BcsCodec {};
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        let kernel: MockKernel<TestSpec> = MockKernel::new(4, 1);

        let checkpoint = StateCheckpoint::<TestSpec>::new(storage, &kernel);
        let mut scratchpad = checkpoint.to_tx_scratchpad();

        // Save some values in the scratchpad.
        let storage_key_1 = SlotKey::from(vec![1]);
        let storage_value_1 = SlotValue::new(&vec![11], &codec);
        save_and_check_value(&storage_key_1, storage_value_1.clone(), &mut scratchpad);

        let gas_meter = BasicGasMeter::new_with_gas(
            <<TestSpec as Spec>::Gas as crate::Gas>::max(),
            <<TestSpec as Spec>::Gas as crate::Gas>::Price::ZEROED,
        );
        let mut pre_exec_ws = scratchpad.to_pre_exec_working_set(gas_meter);

        assert_eq!(
            Some(storage_value_1.clone()),
            StateReader::<User>::get(&mut pre_exec_ws, &storage_key_1)
                .expect("This should be unfaillible")
        );

        // Save some values in the pre_exec_ws
        let storage_key_2 = SlotKey::from(vec![2]);
        let storage_value_2 = SlotValue::new(&vec![22], &codec);
        save_and_check_value(&storage_key_2, storage_value_2.clone(), &mut pre_exec_ws);

        // Commit changes
        let mut pre_exec_ws = pre_exec_ws.commit();

        // Save some values in the pre_exec_ws
        let storage_key_3 = SlotKey::from(vec![3]);
        let storage_value_3 = SlotValue::new(&vec![33], &codec);
        save_and_check_value(&storage_key_3, storage_value_3.clone(), &mut pre_exec_ws);

        let (mut new_scratchpad, _) = pre_exec_ws.revert();

        // After reverting, only the values set before the `commit` should be visible.
        assert_eq!(
            Some(storage_value_1),
            StateReader::<User>::get(&mut new_scratchpad, &storage_key_1)
                .expect("This should be unfaillible")
        );

        assert_eq!(
            Some(storage_value_2),
            StateReader::<User>::get(&mut new_scratchpad, &storage_key_2)
                .expect("This should be unfaillible")
        );

        assert_eq!(
            None,
            StateReader::<User>::get(&mut new_scratchpad, &storage_key_3)
                .expect("This should be unfaillible")
        );
    }
}

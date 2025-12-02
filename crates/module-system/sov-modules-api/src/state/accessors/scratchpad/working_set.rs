//! Working set implementation for transaction execution.

#[cfg(test)]
use std::marker::PhantomData;

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_state::pinned_cache::PinnedCache;
use sov_state::{EventContainer, Namespace, SlotKey, SlotValue, TypeErasedEvent};

use super::super::checkpoints::StateCheckpoint;
use super::super::internals::RevertableWriter;
use super::super::temp_cache::TempCache;
use super::super::{
    BorshSerializedSize, StateMetricsProvider, StateProvider, UniversalStateAccessor,
};
use super::TxScratchpad;
use crate::module::Spec;
use crate::state::traits::{delegate_version_reader, PerBlockCache, PinnedCacheAccessor};
use crate::transaction::{
    transaction_consumption_helper, AuthenticatedTransactionData, PriorityFeeBips,
    TransactionConsumption,
};
use crate::{Amount, BasicGasMeter, Gas, GasInfo, GasMeter, GasMeteringError, GetGasPrice};

#[cfg(feature = "test-utils")]
use crate::GasArray;

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
            gas_price,
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

impl<S: Spec, I: StateProvider<S> + StateMetricsProvider> StateMetricsProvider
    for WorkingSet<S, I>
{
    fn metrics(&mut self) -> &mut StateMetrics {
        self.delta.metrics()
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
            StateCheckpoint::new(inner, &MockKernel::<S>::default(), None);
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
                *price,
            ),
            max_fee: Amount::ZERO,
            max_priority_fee_bips: PriorityFeeBips::ZERO,
        }
    }

    /// Creates a new [`WorkingSet`] instance backed by the given [`Spec::Storage`] and a [`Kernel`].
    pub fn new_with_kernel<K: Kernel<S>>(inner: S::Storage, kernel: &K) -> Self {
        let state_checkpoint: StateCheckpoint<S> = StateCheckpoint::new(inner, kernel, None);
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

    fn charge_gas(&mut self, gas: S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        self.gas_meter.charge_gas(gas)
    }

    fn try_as_basic_gas_meter(&mut self) -> Option<&mut BasicGasMeter<Self::Spec>> {
        self.gas_meter.try_as_basic_gas_meter()
    }

    fn charge_linear_gas(
        &mut self,
        amount: <Self::Spec as Spec>::Gas,
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
    fn gas_price(&self) -> <S::Gas as Gas>::Price {
        self.gas_meter.gas_price()
    }
}

impl<S: Spec, I: StateProvider<S>> UniversalStateAccessor for WorkingSet<S, I> {
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<u32> {
        self.delta.get_size(namespace, key, metrics)
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        self.delta.get_value(namespace, key, metrics)
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

delegate_version_reader!(WorkingSet<S, I> where [S: Spec, I: StateProvider<S>] => delta.inner);

impl<S: Spec, I: StateProvider<S>> PerBlockCache for WorkingSet<S, I> {
    fn get_cached<T: 'static + Send + Sync>(&self, slot_key: Option<SlotKey>) -> Option<&T> {
        self.delta.get_cached::<T>(slot_key)
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(
        &mut self,
        slot_key: Option<SlotKey>,
        value: T,
    ) {
        self.delta.cache_writes.set(slot_key, value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self, slot_key: Option<SlotKey>) {
        self.delta.cache_writes.delete::<T>(slot_key);
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.delta.cache_writes.update_with(other);
    }
}

impl<S: Spec, I: StateProvider<S>> PinnedCacheAccessor<S> for WorkingSet<S, I> {
    fn pinned_cache_mut(&mut self) -> Option<&mut PinnedCache> {
        self.delta.inner.pinned_cache_mut()
    }

    fn storage(&self) -> &S::Storage {
        self.delta.inner.storage()
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
    use crate::{StateCheckpoint, StateReader, StateWriter, WorkingSet};

    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn test_workingset_get() {
        let codec = BcsCodec {};
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();

        let prefix = sov_state::Prefix::new(1, 2);
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

        let prefix = sov_state::Prefix::new(1, 2);
        let storage_key = SlotKey::new::<HexString, _, _>(&prefix, [4, 5, 6].as_ref(), &codec);
        let storage_value = SlotValue::new(&vec![7, 8, 9], &codec);
        let kernel: MockKernel<TestSpec> = MockKernel::new(4, 1);

        let mut working_set = StateCheckpoint::<TestSpec>::new(storage.clone(), &kernel, None);
        let mut working_set = kernel.accessor(&mut working_set);

        StateWriter::<Kernel>::set(&mut working_set, &storage_key, storage_value.clone())
            .expect("This should be unfaillible");

        assert_eq!(
            Some(storage_value),
            StateReader::<Kernel>::get(&mut working_set, &storage_key)
                .expect("This should be unfaillible")
        );
    }
}

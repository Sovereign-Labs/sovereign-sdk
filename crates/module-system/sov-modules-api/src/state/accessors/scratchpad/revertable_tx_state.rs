//! Revertable transaction state implementation.

use std::collections::HashMap;
use std::marker::PhantomData;

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_state::pinned_cache::PinnedCache;
use sov_state::{
    EventContainer, Kernel as KernelType, Namespace, SlotKey, SlotValue, TypeErasedEvent, User,
};

use super::super::temp_cache::{CacheLookup, TempCache};
use super::super::{BorshSerializedSize, StateMetricsProvider, UniversalStateAccessor};
use crate::module::Spec;
use crate::state::traits::PerBlockCache;
use crate::state::traits::{delegate_version_reader, PinnedCacheAccessor};
use crate::{
    AccessoryStateWriter, BasicGasMeter, GasMeter, GasMeteringError, ProvableStateReader,
    ProvableStateWriter, TxState,
};

#[cfg(feature = "test-utils")]
use crate::AccessoryStateReader;
/// A revertable state layer that wraps a [`TxState`] and tracks writes and events separately.
///
/// Changes can be either committed to the underlying state via [`RevertableTxState::commit`],
/// or discarded via [`RevertableTxState::revert`].
///
/// ## Usage note
/// This structure tracks gas consumed outside of the transaction lifecycle without explicitly consuming a finite resource.
/// This should only be used in infallible methods.
pub struct RevertableTxState<'a, S: Spec, State> {
    pub(super) inner: &'a mut State,
    pub(super) events: Vec<TypeErasedEvent>,
    pub(super) temp_cache: TempCache,
    pub(super) writes: HashMap<(Namespace, SlotKey), Option<SlotValue>>,
    pub(super) phantom: PhantomData<S>,
}

impl<S: Spec, I: StateMetricsProvider> StateMetricsProvider for RevertableTxState<'_, S, I> {
    fn metrics(&mut self) -> &mut StateMetrics {
        self.inner.metrics()
    }
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
    fn get_cached<T: 'static + Send + Sync>(&self, slot_key: Option<SlotKey>) -> Option<&T> {
        match self.temp_cache.get::<T>(slot_key.clone()) {
            CacheLookup::Hit(value) => value,
            CacheLookup::Miss => self.inner.get_cached::<T>(slot_key),
        }
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(
        &mut self,
        slot_key: Option<SlotKey>,
        value: T,
    ) {
        self.temp_cache.set(slot_key, value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self, slot_key: Option<SlotKey>) {
        self.temp_cache.delete::<T>(slot_key);
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.temp_cache.update_with(other);
    }
}

impl<S: Spec, I: TxState<S>> PinnedCacheAccessor<S> for RevertableTxState<'_, S, I> {
    fn pinned_cache_mut(&mut self) -> Option<&mut PinnedCache> {
        self.inner.pinned_cache_mut()
    }

    fn storage(&self) -> &S::Storage {
        self.inner.storage()
    }
}

delegate_version_reader!(RevertableTxState<'_, S, I> where [S: Spec, I: TxState<S>] => inner);

impl<S: Spec, I: TxState<S>> UniversalStateAccessor for RevertableTxState<'_, S, I> {
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<u32> {
        if let Some(value) = self.writes.get(&(namespace, key.clone())) {
            return value.as_ref().map(|v| v.size());
        }
        self.inner.get_size(namespace, key, metrics)
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        if let Some(value) = self.writes.get(&(namespace, key.clone())) {
            return value.clone();
        }
        self.inner.get_value(namespace, key, metrics)
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
    fn charge_gas(&mut self, amount: S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        self.inner.charge_gas(amount)
    }
    fn try_as_basic_gas_meter(&mut self) -> Option<&mut BasicGasMeter<Self::Spec>> {
        self.inner.try_as_basic_gas_meter()
    }

    fn charge_linear_gas(
        &mut self,
        amount: <Self::Spec as Spec>::Gas,
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
    fn add_event<E: 'static + core::marker::Send + core::marker::Sync>(&mut self, event_key: &str, event: E) {
        self.events.push(TypeErasedEvent::new(event_key, event));
    }

    fn add_type_erased_event(&mut self, event: TypeErasedEvent) {
        self.events.push(event);
    }
}

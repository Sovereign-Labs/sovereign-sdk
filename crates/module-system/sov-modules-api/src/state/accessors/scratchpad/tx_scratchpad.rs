//! Transaction scratchpad implementation.

use std::marker::PhantomData;

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_state::{Namespace, SlotKey, SlotValue};

use super::super::checkpoints::StateCheckpoint;
use super::super::internals::RevertableWriter;
use super::super::temp_cache::TempCache;
use super::super::{
    BorshSerializedSize, StateMetricsProvider, StateProvider, UniversalStateAccessor,
};
use super::PreExecWorkingSet;
use crate::module::Spec;
use crate::state::traits::{delegate_version_reader, PerBlockCache};
use crate::{BasicGasMeter, GasMeter};

/// Transaction-level state accumulator without gas metering.
///
/// This structure is built from a [`StateProvider`] (typically a [`StateCheckpoint`])
/// and accumulates all state changes from a transaction. It can be converted to a
/// [`PreExecWorkingSet`] or [`WorkingSet`] when gas metering is needed.
///
/// ## Usage note
/// This structure does not meter gas - it implements [`GasMeter`] as a no-op.
/// Use [`PreExecWorkingSet`] or [`WorkingSet`] for actual gas metering.
pub struct TxScratchpad<S: Spec, I: StateProvider<S>> {
    pub(in crate::state::accessors) inner: RevertableWriter<I>,
    pub(in crate::state::accessors) phantom: PhantomData<S>,
}

impl<S: Spec, I: StateProvider<S>> UniversalStateAccessor for TxScratchpad<S, I> {
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<u32> {
        <RevertableWriter<I> as UniversalStateAccessor>::get_size(
            &mut self.inner,
            namespace,
            key,
            metrics,
        )
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        <RevertableWriter<I> as UniversalStateAccessor>::get_value(
            &mut self.inner,
            namespace,
            key,
            metrics,
        )
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
pub struct TxChangeSet {
    /// The transaction writes.
    pub writes: Vec<((SlotKey, sov_state::Namespace), Option<SlotValue>)>,
}

impl<S: Spec, I: StateProvider<S>> TxScratchpad<S, I> {
    /// Commits the changes of this [`TxScratchpad`] and returns a [`StateCheckpoint`].
    pub fn commit(self) -> I {
        self.inner.commit()
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

impl<S: Spec, I: StateProvider<S> + StateMetricsProvider> StateMetricsProvider
    for TxScratchpad<S, I>
{
    fn metrics(&mut self) -> &mut StateMetrics {
        self.inner.metrics()
    }
}

delegate_version_reader!(TxScratchpad<S, I> where [S: Spec, I: StateProvider<S>] => inner.inner);

impl<S: Spec, I: StateProvider<S>> PerBlockCache for TxScratchpad<S, I> {
    fn get_cached<T: 'static + Send + Sync>(&self, slot_key: Option<SlotKey>) -> Option<&T> {
        self.inner.get_cached::<T>(slot_key)
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(
        &mut self,
        slot_key: Option<SlotKey>,
        value: T,
    ) {
        self.inner.cache_writes.set(slot_key, value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self, slot_key: Option<SlotKey>) {
        self.inner.cache_writes.delete::<T>(slot_key);
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.inner.cache_writes.update_with(other);
    }
}

impl<S: Spec> TxScratchpad<S, StateCheckpoint<S>> {
    /// Change set resulting from transaction execution.
    pub fn tx_changes(&self) -> TxChangeSet {
        self.inner.changes()
    }
}

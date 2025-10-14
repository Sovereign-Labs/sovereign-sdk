//! Transaction scratchpad implementation.

use std::marker::PhantomData;

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_rollup_interface::stf::ExecutionContext;
use sov_state::{Namespace, NodeLeafAndMaybeValue, SlotKey, SlotValue};

use super::super::checkpoints::StateCheckpoint;
use super::super::internals::{FirstTimeReads, RevertableWriter};
use super::super::temp_cache::TempCache;
use super::super::{BorshSerializedSize, StateMetricsProvider, StateProvider, UniversalStateAccessor};
use super::PreExecWorkingSet;
use crate::capabilities::RollupHeight;
use crate::module::Spec;
use crate::state::traits::PerBlockCache;
use crate::{BasicGasMeter, GasMeter, VersionReader};

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
    /// The transaction reads.
    pub reads: Option<FirstTimeReads>,
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
    pub fn tx_changes(&self, execution_context: ExecutionContext) -> TxChangeSet {
        self.inner.changes(execution_context)
    }

    /// Applies changes to the TxScratchpad to warm up its cache.
    pub fn apply_change_set(&mut self, changes: TxChangeSet) {
        let mut reads = changes.reads.unwrap();

        // If a write overrides a read, ignore the read.
        self.filter_out_writes(&mut reads.user, Namespace::User);
        self.filter_out_writes(&mut reads.kernel, Namespace::Kernel);

        self.inner.inner.add_read_if_not_present(reads);
    }

    fn filter_out_writes(
        &self,
        reads: &mut Vec<(SlotKey, Option<NodeLeafAndMaybeValue>)>,
        namespace: Namespace,
    ) {
        reads.retain(|(k, _)| !self.inner.writes.contains_key(&(k.clone(), namespace)));
    }
}

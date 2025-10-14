//! Transaction scratchpad implementation.

use std::marker::PhantomData;

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_rollup_interface::stf::ExecutionContext;
use sov_state::{Namespace, NodeLeafAndMaybeValue, SlotKey, SlotValue};

use super::super::checkpoints::StateCheckpoint;
use super::super::internals::{FirstTimeReads, RevertableWriter};
use super::super::temp_cache::TempCache;
use super::super::{
    BorshSerializedSize, StateMetricsProvider, StateProvider, UniversalStateAccessor,
};
use super::PreExecWorkingSet;
use crate::capabilities::RollupHeight;
use crate::module::Spec;
use crate::state::traits::{delegate_version_reader, PerBlockCache};
use crate::{BasicGasMeter, GasMeter, VersionReader};

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

#[cfg(test)]
mod tests {
    use sov_metrics::StateAccessMetric;
    use sov_rollup_interface::stf::ExecutionContext;
    use sov_state::codec::BcsCodec;
    use sov_state::{Namespace, NodeLeafAndMaybeValue, SlotKey, SlotValue};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::TestHasher;
    use sov_test_utils::{MockDaSpec, MockZkvm};

    use super::{TxChangeSet, TxScratchpad};
    use crate::capabilities::mocks::MockKernel;
    use crate::execution_mode::Native;
    use crate::state::accessors::internals::FirstTimeReads;
    use crate::state::accessors::seal::UniversalStateAccessor;
    use crate::{Spec, StateCheckpoint, StateProvider};

    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn test_cache_warmup_changeset() {
        test_cache_warmup_changeset_with_namespace(Namespace::User);
        test_cache_warmup_changeset_with_namespace(Namespace::Kernel);
    }

    fn test_cache_warmup_changeset_with_namespace(namespace: Namespace) {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();

        let mut worker_scratchpad = create_srcratchpad::<TestSpec>(storage.clone());

        // Key and values.
        let init_reads = vec![(10, 1), (11, 2), (12, 3)];

        // Simulate a worker reading some values and filling its cache.
        {
            worker_scratchpad.apply_change_set(changeset(init_reads.clone(), namespace));
            assert_scratchpad_contains_data(&init_reads, &mut worker_scratchpad, namespace);
        }

        // Only `ExecutionContext::SequencerWarmUp` is responsible for placing the reads into `TxChangeSet``.
        {
            let ch = worker_scratchpad.tx_changes(ExecutionContext::Sequencer);
            assert!(ch.reads.is_none());

            let ch = worker_scratchpad.tx_changes(ExecutionContext::Node);
            assert!(ch.reads.is_none());
        }

        // Retrieve the changeset from the worker, then apply it to the main scratchpad.
        let changeset_from_worker = worker_scratchpad.tx_changes(ExecutionContext::SequencerWarmUp);
        let mut main_scratchpad = create_srcratchpad::<TestSpec>(storage.clone());

        // After receiving the changeset from the worker, the main scratchpad can see all its values.
        {
            main_scratchpad.apply_change_set(changeset_from_worker.clone());
            assert_scratchpad_contains_data(&init_reads, &mut main_scratchpad, namespace);
        }
    }

    #[test]
    fn test_cache_warmup_overrides() {
        test_cache_warmup_overrides_with_namespace(Namespace::User);
        test_cache_warmup_overrides_with_namespace(Namespace::Kernel);
    }

    fn test_cache_warmup_overrides_with_namespace(namespace: Namespace) {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();

        let mut worker_scratchpad = create_srcratchpad::<TestSpec>(storage.clone());
        let mut main_scratchpad = create_srcratchpad::<TestSpec>(storage.clone());

        // Initialize the main_scratchpad cache with reads
        let main_reads = vec![(10, 1), (11, 2), (12, 3)];
        main_scratchpad.apply_change_set(changeset(main_reads.clone(), namespace));

        // Initialize the main_scratchpad cache with writes.
        let main_writes = vec![(20, 11), (21, 12), (22, 13)];
        for (k, v) in main_writes.clone() {
            main_scratchpad.set_value(namespace, &key(k), value(v));
        }

        let changeset_from_worker = {
            let worker_reads = vec![
                // These keys are the same as the read/write keys observed by the main scratchpad above, but they hold different values.
                (10, 0),
                (11, 0),
                (12, 0),
                (20, 0),
                (21, 0),
                (22, 0),
                // These key–value pairs are extra and not present in the main scratchpad.
                (101, 111),
                (102, 112),
            ];

            worker_scratchpad.apply_change_set(changeset(worker_reads, namespace));
            worker_scratchpad.tx_changes(ExecutionContext::SequencerWarmUp)
        };
        main_scratchpad.apply_change_set(changeset_from_worker);

        // The main_reads and main_writes were not overridden by the changes from the worker_scratchpad.
        assert_scratchpad_contains_data(&main_reads, &mut main_scratchpad, namespace);
        assert_scratchpad_contains_data(&main_writes, &mut main_scratchpad, namespace);
        // The extra key–value pairs are inserted into the main_scratchpad.
        assert_scratchpad_contains_data(&[(101, 111), (102, 112)], &mut main_scratchpad, namespace);
    }

    fn create_srcratchpad<S: Spec>(storage: S::Storage) -> TxScratchpad<S, StateCheckpoint<S>> {
        let checkpoint = StateCheckpoint::new(storage, &MockKernel::new(4, 1));
        checkpoint.to_tx_scratchpad()
    }

    fn key(k: u8) -> SlotKey {
        SlotKey::test_key(k)
    }

    fn value(v: u8) -> SlotValue {
        SlotValue::new(&vec![v], &BcsCodec {})
    }

    fn changeset(reads: Vec<(u8, u8)>, namespace: Namespace) -> TxChangeSet {
        let reads = reads
            .into_iter()
            .map(|(k, v)| {
                (
                    key(k),
                    Some(NodeLeafAndMaybeValue::new_read::<TestHasher>(value(v))),
                )
            })
            .collect();

        let (user, kernel) = match namespace {
            Namespace::User => (reads, vec![]),
            Namespace::Kernel => (vec![], reads),
            Namespace::Accessory => unimplemented!(),
        };

        TxChangeSet {
            writes: vec![],
            reads: Some(FirstTimeReads { user, kernel }),
        }
    }

    fn assert_scratchpad_contains_data(
        data: &[(u8, u8)],
        scratchpad: &mut TxScratchpad<TestSpec, StateCheckpoint<TestSpec>>,
        namespace: Namespace,
    ) {
        let mut metric = StateAccessMetric::placeholder();
        for (k, v) in data {
            let get_value = scratchpad
                .get_value(namespace, &key(*k), &mut metric)
                .unwrap();
            assert_eq!(value(*v), get_value);
        }
    }
}

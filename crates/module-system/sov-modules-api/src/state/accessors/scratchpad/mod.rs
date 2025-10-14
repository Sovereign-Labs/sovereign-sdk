//! Transaction state management structures.
//!
//! This module contains types for managing state during transaction execution:
//! - [`RevertableTxState`]: Temporary state changes within a transaction that can be committed or reverted
//! - [`TxScratchpad`]: Transaction-level state diff without gas metering
//! - [`PreExecWorkingSet`]: Pre-execution working set with gas metering for checks
//! - [`WorkingSet`]: Full transaction execution context with gas metering and events
//! - [`TxChangeSet`]: Records of changes from transaction execution

mod revertable_tx_state;
mod tx_scratchpad;
mod pre_exec_working_set;
mod working_set;

pub use revertable_tx_state::RevertableTxState;
pub use tx_scratchpad::{TxChangeSet, TxScratchpad};
pub use pre_exec_working_set::PreExecWorkingSet;
pub use working_set::WorkingSet;

#[cfg(test)]
mod tests {
    use sov_metrics::StateAccessMetric;
    use sov_rollup_interface::common::HexString;
    use sov_rollup_interface::stf::ExecutionContext;
    use sov_state::codec::BcsCodec;
    use sov_state::namespaces::User;
    use sov_state::Namespace;
    use sov_state::{Kernel, NodeLeafAndMaybeValue, SlotKey, SlotValue};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::TestHasher;
    use sov_test_utils::{MockDaSpec, MockZkvm};

    use crate::capabilities::mocks::MockKernel;
    use crate::capabilities::Kernel as _;
    use crate::execution_mode::Native;
    use crate::state::accessors::internals::FirstTimeReads;
    use crate::state::accessors::seal::UniversalStateAccessor;
    use crate::{
        BasicGasMeter, GasArray, Spec, StateAccessor, StateCheckpoint, StateProvider, StateReader,
        StateWriter, TxChangeSet, TxScratchpad, WorkingSet,
    };

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
        let storage_key_1 = SlotKey::test_key(1);
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
        let storage_key_2 = SlotKey::test_key(2);
        let storage_value_2 = SlotValue::new(&vec![22], &codec);
        save_and_check_value(&storage_key_2, storage_value_2.clone(), &mut pre_exec_ws);

        // Commit changes
        let mut pre_exec_ws = pre_exec_ws.commit();

        // Save some values in the pre_exec_ws
        let storage_key_3 = SlotKey::test_key(3);
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

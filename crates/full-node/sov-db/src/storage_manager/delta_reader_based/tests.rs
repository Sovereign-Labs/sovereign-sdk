use std::path::Path;

use jmt::storage::HasPreimage;
use rockbound::SchemaBatch;
use sov_db::accessory_db::AccessoryDb;
use sov_mock_da::{MockDaSpec, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::storage::HierarchicalStorageManager;

use crate::namespaces::{KernelNamespace, Namespace, UserNamespace};
use crate::state_db::StateDb;
use crate::storage_manager::delta_reader_based::{NativeChangeSet, NativeStorageManager};
use crate::storage_manager::tests::arbitrary::ForkDescription;
use crate::storage_manager::tests::data_helpers::{verify_accessory_db, H};
use crate::storage_manager::tests::generic_tests::{
    calls_on_empty, check_snapshots_ordering, create_state_after_not_saved_block,
    double_create_storage, double_save_changes, finalize_only_last_block,
    ledger_finalized_height_is_updated_on_start, linear_progression, minimal_fork_bfs,
    parallel_forks_reading_while_finalization_is_happening, removed_fork_data_view,
    several_jumping_forks, test_exploration, unknown_block_cannot_be_saved, ExplorationMode,
    TestableStorage, TestableStorageManager,
};
use crate::test_utils::{build_data_to_materialize, TestNativeStorage};

impl TestableStorage for TestNativeStorage {
    type ChangeSet = NativeChangeSet;

    fn materialize_from_key_values(self, items: &[(Vec<u8>, Option<Vec<u8>>)]) -> Self::ChangeSet {
        let mut preimages = Vec::with_capacity(items.len());
        let mut batch = Vec::with_capacity(items.len());
        let mut accessory_batch = Vec::with_capacity(items.len());

        for (key, value) in items {
            let key_hash = jmt::KeyHash::with::<H>(&key);
            preimages.push((key_hash, key));
            batch.push((key_hash, value.clone()));
            accessory_batch.push((key.clone(), value.clone()));
        }

        let materialized_preimages =
            StateDb::materialize_preimages(preimages.clone(), preimages.clone()).unwrap();

        let jmt_handler_user = self.state.get_jmt_handler::<UserNamespace>();
        let jmt_handler_kernel = self.state.get_jmt_handler::<KernelNamespace>();

        let data_to_materialize_user = build_data_to_materialize::<_, H>(
            &jmt_handler_user,
            SlotNumber::GENESIS.get(),
            batch.clone(),
        );
        let data_to_materialize_kernel = build_data_to_materialize::<_, H>(
            &jmt_handler_kernel,
            SlotNumber::GENESIS.get(),
            batch.clone(),
        );

        let state_change_set = self
            .state
            .materialize(
                &data_to_materialize_kernel,
                &data_to_materialize_user,
                Some(materialized_preimages),
            )
            .unwrap();

        let accessory_change_set =
            AccessoryDb::materialize_values(accessory_batch, SlotNumber::GENESIS).unwrap();

        NativeChangeSet {
            state_change_set,
            accessory_change_set,
        }
    }

    fn get_value(&self, key: &[u8]) -> Option<Vec<u8>> {
        let user_value = self
            .state
            .get_value_option_by_key::<UserNamespace>(SlotNumber::GENESIS, &key.to_vec())
            .unwrap();
        let kernel_value = self
            .state
            .get_value_option_by_key::<KernelNamespace>(SlotNumber::GENESIS, &key.to_vec())
            .unwrap();
        assert_eq!(user_value, kernel_value);
        user_value
    }
}

type Sm = NativeStorageManager<MockDaSpec, TestNativeStorage>;

impl TestableStorageManager for Sm {
    fn new(path: impl AsRef<Path>) -> Self {
        NativeStorageManager::new(path).unwrap()
    }

    fn verify_stf_storage(stf_storage: &Self::StfState, expected_values: &[(u64, MockHash)]) {
        verify_stf_storage(stf_storage, expected_values);
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn snapshots_count(&self) -> usize {
        self.snapshots_count()
    }

    fn blocks_to_parent_count(&self) -> usize {
        self.blocks_to_parent_count()
    }
}

fn verify_stf_storage(stf_storage: &TestNativeStorage, expected_values: &[(u64, MockHash)]) {
    verify_state_db::<UserNamespace>(&stf_storage.state, expected_values);
    verify_state_db::<KernelNamespace>(&stf_storage.state, expected_values);
    verify_accessory_db(&stf_storage.accessory_db, expected_values);
}

// We cannot check that extra data hasn't been written,
// because StateDb does not expose range API, but this is highly unlikely that some data is copied.
// And it will be better tested by integration tests with business logic.
// This checks at least test presence of the expected data.
fn verify_state_db<N: Namespace>(state_db: &StateDb, expected_values: &[(u64, MockHash)]) {
    let jmt_handler = state_db.get_jmt_handler::<N>();
    for (expected_height, expected_hash) in expected_values {
        let height_bytes = expected_height.to_be_bytes();
        let key_hash = jmt::KeyHash::with::<H>(&height_bytes);
        let pre_image = jmt_handler
            .preimage(key_hash)
            .unwrap()
            .expect("Missing preimage");
        assert_eq!(&pre_image, &height_bytes);
        let value = state_db
            .get_value_option_by_key::<N>(SlotNumber::GENESIS, &pre_image)
            .expect("Failed to get value option from state db");
        assert_eq!(Some(expected_hash.0.to_vec()), value);
    }
}

#[test_log::test]
fn test_delta_reader_based_storage_manager_linear_progression() {
    // Instant finality
    linear_progression::<Sm>(5, 0);
    // Non-instant finality
    linear_progression::<Sm>(5, 1);
    linear_progression::<Sm>(5, 4);
    linear_progression::<Sm>(5, 5);
    linear_progression::<Sm>(5, 6);
    linear_progression::<Sm>(5, 10);
}

#[test_log::test]
fn delta_reader_minimal_fork_bfs() {
    minimal_fork_bfs::<Sm>();
}

#[test_strategy::proptest]
fn proptest_forks_exploration(fork: ForkDescription) {
    test_exploration::<Sm>(fork.clone(), ExplorationMode::Bfs);
    test_exploration::<Sm>(fork, ExplorationMode::Dfs);
}

#[test]
fn test_calls_on_empty() {
    calls_on_empty::<Sm>();
}

#[test]
fn test_double_create_storage() {
    double_create_storage::<Sm>();
}

#[test]
fn test_unknown_block_cannot_be_saved() {
    unknown_block_cannot_be_saved::<Sm>();
}

#[test]
fn test_double_save_changes() {
    double_save_changes::<Sm>();
}

#[test]
fn test_create_state_after_not_saved_block() {
    create_state_after_not_saved_block::<Sm>();
}

#[test]
fn test_finalize_only_last_block() {
    finalize_only_last_block::<Sm>();
}

// TODO: Needs to be converted to benchmark
#[test]
fn flaky_test_parallel_forks_reading_while_finalization_is_happening() {
    parallel_forks_reading_while_finalization_is_happening::<Sm>();
}

#[test]
fn test_several_jumping_forks() {
    several_jumping_forks::<Sm>();
}

#[test]
fn test_removed_fork_view() {
    removed_fork_data_view::<Sm>();
}

// This test is similar to `removed_fork_data_view`,
// But it demonstrates that change happens only to data that has been in rocksdb before the block is created.
// Data that has been in the snapshot when a block has been created remains the same for the fork.
#[test]
fn test_fork_keeps_reference_to_snapshot_after_finalization() {
    let key = 1u64.to_be_bytes();

    let value_1 = vec![1u8; 32];
    let value_2 = [2u8; 32];
    // Just to put data into rocksdb
    let (block_a, block_b, block_c) =
        crate::storage_manager::tests::generic_tests::get_parent_and_2_children();

    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager: Sm = NativeStorageManager::new(tmpdir.path()).unwrap();
    let (storage, _) = storage_manager.create_state_for(&block_a).unwrap();

    let stf_changes = storage.materialize_from_key_value(key.to_vec(), Some(value_1.to_vec()));
    storage_manager
        .save_change_set(&block_a, stf_changes, SchemaBatch::default())
        .unwrap();

    let (stf_reader_b, _) = storage_manager.create_state_for(&block_b).unwrap();
    let (stf_reader_c, _) = storage_manager.create_state_for(&block_c).unwrap();

    // Bump version, so the reader works
    // let key = encode_state_key_with_version(1, 1);
    let value_at_b = stf_reader_b.get_value(&key);
    assert_eq!(Some(value_1.clone()), value_at_b);
    let value_at_c = stf_reader_c.get_value(&key);
    assert_eq!(Some(value_1.clone()), value_at_c);
    let stf_changes = stf_reader_b.materialize_from_key_value(key.to_vec(), Some(value_2.to_vec()));
    storage_manager
        .save_change_set(&block_b, stf_changes, SchemaBatch::default())
        .unwrap();

    storage_manager.finalize(&block_a).unwrap();
    storage_manager.finalize(&block_b).unwrap();

    // reader_c still observes data from block A,
    // even though it has been overwritten in rocksdb by block B
    let value_at_c = stf_reader_c.get_value(&key);
    assert_eq!(Some(value_1), value_at_c);
}

#[test]
fn test_snapshots_ordering() {
    check_snapshots_ordering::<Sm>();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ledger_finalized_height_is_updated_on_start() {
    ledger_finalized_height_is_updated_on_start::<Sm>().await;
}

use std::path::Path;
use std::sync::Arc;

use nomt::trie::KeyPath;
use rockbound::{SchemaBatch, SchemaValue};
use sha2::Digest;
use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::storage::HierarchicalStorageManager;

use super::{NomtChangeSet, NomtStorageManager, StateFinishedSession};
use crate::accessory_db::AccessoryDb;
use crate::config::RollupDbConfig;
use crate::historical_state::HistoricalStateReader;
use crate::namespaces::{KernelNamespace, UserNamespace};
use crate::state_db_nomt::{get_session_builder_from_committed, NomtStateDb, StateRootHashes};
use crate::storage_manager::tests::arbitrary::ForkDescription;
use crate::storage_manager::tests::data_helpers::verify_accessory_db;
use crate::storage_manager::tests::generic_tests::{
    calls_on_empty, check_snapshots_ordering, create_state_after_not_saved_block,
    double_create_storage, double_save_changes, finalize_only_last_block,
    ledger_finalized_height_is_updated_on_start, linear_progression, minimal_fork_bfs,
    parallel_forks_reading_while_finalization_is_happening, removed_fork_data_view,
    several_jumping_forks, test_exploration, unknown_block_cannot_be_saved, ExplorationMode,
    TestableStorage, TestableStorageManager,
};
use crate::test_utils::{TestNomtStorage, H};

impl std::fmt::Debug for TestNomtStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestNomtStorage")
            .field("state_session", &"<StateSession>")
            .field("accessory_db", &self.accessory_db)
            .finish()
    }
}

impl TestableStorage for TestNomtStorage {
    type ChangeSet = NomtChangeSet;

    fn materialize_from_key_values(self, items: &[(Vec<u8>, Option<Vec<u8>>)]) -> Self::ChangeSet {
        let TestNomtStorage {
            state_session_builder,
            historical_state: _,
            accessory_db: _,
        } = self;

        let user_session = state_session_builder.begin_user_session().unwrap();
        let kernel_session = state_session_builder.begin_kernel_session().unwrap();

        let mut state_writes = Vec::with_capacity(items.len());
        let mut accessory_writes = Vec::with_capacity(items.len());

        for (key, value) in items {
            let key_path = KeyPath::from(sha2::Sha256::digest(key));
            kernel_session.warm_up(key_path);
            user_session.warm_up(key_path);
            state_writes.push((key_path, nomt::KeyReadWrite::Write(value.clone())));
            accessory_writes.push((key.clone(), value.clone()));
        }

        state_writes.sort_by_key(|(k, _)| *k);

        let user_finished_session = user_session.finish(state_writes.clone()).unwrap();
        let kernel_finished_session = kernel_session.finish(state_writes.clone()).unwrap();

        let accessory_change_set =
            AccessoryDb::materialize_values(accessory_writes.clone(), SlotNumber::GENESIS).unwrap();

        let historical_change_set = HistoricalStateReader::materialize_values(
            accessory_writes.clone(),
            accessory_writes.clone(),
            // Not used at the moment,
            items.len().to_be_bytes().to_vec(),
            SlotNumber::GENESIS,
        )
        .unwrap();

        NomtChangeSet {
            state: StateFinishedSession {
                user: user_finished_session,
                kernel: kernel_finished_session,
            },
            historical_state: historical_change_set,
            accessory: accessory_change_set,
        }
    }

    fn get_value(&self, key: &[u8]) -> Option<Vec<u8>> {
        let schema_key = key.to_vec();
        let key_path = KeyPath::from(sha2::Sha256::digest(key));
        let kernel_value = {
            let kernel_session = self.state_session_builder.begin_kernel_session().unwrap();
            kernel_session.read(key_path).unwrap()
        };
        let user_value = {
            let user_session = self.state_session_builder.begin_user_session().unwrap();
            user_session.read(key_path).unwrap()
        };
        assert_eq!(kernel_value, user_value);

        let accessory_value = self
            .accessory_db
            .get_value_option(&schema_key, SlotNumber::GENESIS)
            .unwrap();
        assert_eq!(accessory_value, kernel_value);

        let historical_value_user = self
            .historical_state
            .get_value_option_by_key::<UserNamespace>(SlotNumber::GENESIS, &schema_key)
            .unwrap();
        assert_eq!(historical_value_user, kernel_value);

        let historical_value_kernel = self
            .historical_state
            .get_value_option_by_key::<KernelNamespace>(SlotNumber::GENESIS, &schema_key)
            .unwrap();
        assert_eq!(historical_value_kernel, kernel_value);

        kernel_value
    }
}

type Sm = NomtStorageManager<MockDaSpec, H, TestNomtStorage>;

impl TestableStorageManager for Sm {
    fn new(path: impl AsRef<Path>) -> Self {
        let config = RollupDbConfig::default_in_path(path.as_ref().to_path_buf());
        Sm::new(config).unwrap()
    }

    fn verify_stf_storage(stf_storage: &Self::StfState, expected_values: &[(u64, MockHash)]) {
        for (expected_height, expected_hash) in expected_values {
            let key = expected_height.to_be_bytes().to_vec();
            let actual_value = stf_storage.get_value(&key);
            let expected_value = Some(expected_hash.0.to_vec());
            assert_eq!(actual_value, expected_value);
        }

        // TODO: Verify historical state too!
        verify_accessory_db(&stf_storage.accessory_db, expected_values);
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

#[test_log::test]
fn test_manager_linear_progression() {
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
fn nomt_minimal_fork_bfs() {
    minimal_fork_bfs::<Sm>();
}

#[test_strategy::proptest]
#[ignore = "Too slow on MacOS currently"]
fn proptest_nomt_forks_exploration(fork: ForkDescription) {
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

#[test]
fn test_snapshots_ordering() {
    check_snapshots_ordering::<Sm>();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ledger_finalized_height_is_updated_on_start() {
    ledger_finalized_height_is_updated_on_start::<Sm>().await;
}

/// This grey box test. It relies on knowledge that historical storage is committed the last.
/// It emulates "crash" of historical state commit, by commiting another set of changes to NOMT,
/// So historical state is "lagging behind".
#[tokio::test(flavor = "multi_thread")]
async fn test_root_hashes_match_after_crash() {
    // Create a temporary directory for the test
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().to_path_buf();

    // Initialize storage manager
    let config = RollupDbConfig::default_in_path(db_path.clone());
    let mut storage_manager =
        NomtStorageManager::<MockDaSpec, H, TestNomtStorage>::new(config.clone()).unwrap();

    let blocks: u64 = 10;

    let get_root_hashes = |stf_storage: TestNomtStorage| -> (SchemaValue, StateRootHashes) {
        let last_version = stf_storage.historical_state.last_version().unwrap();
        let historical_root_hash = stf_storage
            .historical_state
            .get_serialized_root_hash(last_version)
            .unwrap()
            .unwrap();
        let (user_session, kernel_session) = stf_storage.begin_sessions();
        let user = user_session.prev_root();
        let kernel = kernel_session.prev_root();
        let state_root_hash = StateRootHashes { user, kernel };
        (historical_root_hash, state_root_hash)
    };

    // Just write (blocks - 1) versions.
    for height in 1u64..blocks {
        let da_header = MockBlockHeader::from_height(height);

        // Create state for the block
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();

        // Materialize some test data
        let user_key = height.to_be_bytes().to_vec();
        let kernel_key = height.to_le_bytes().to_vec();
        let raw_value = da_header.hash.0.to_vec();
        // let stf_changes = stf_storage.materialize_from_key_values(&[(key, Some(value))]);

        let user_key_path = KeyPath::from(sha2::Sha256::digest(user_key.clone()));
        let kernel_key_path = KeyPath::from(sha2::Sha256::digest(kernel_key.clone()));
        let value = nomt::KeyReadWrite::Write(Some(raw_value.clone()));
        let user_nomt_values = vec![(user_key_path, value.clone())];
        let user_historical_values = vec![(user_key, Some(raw_value.clone()))];
        let kernel_nomt_values = vec![(kernel_key_path, value.clone())];
        let kernel_historical_values = vec![(kernel_key, Some(raw_value.clone()))];

        let (user_session, kernel_session) = stf_storage.begin_sessions();

        let user_finished_session = user_session.finish(user_nomt_values).unwrap();
        let kernel_finished_session = kernel_session.finish(kernel_nomt_values).unwrap();

        let user_root_hash = user_finished_session.root().into_inner();
        let kernel_root_hash = kernel_finished_session.root().into_inner();

        // Mimic prover storage.
        // It intentionally not important in which order they are passed,
        // As sov-db remains oblivious about it.
        let root_hash = [user_root_hash, kernel_root_hash].concat();

        let historical_change_set = HistoricalStateReader::materialize_values(
            user_historical_values,
            kernel_historical_values,
            root_hash,
            SlotNumber::new(height),
        )
        .unwrap();

        let stf_changes = NomtChangeSet {
            state: StateFinishedSession {
                user: user_finished_session,
                kernel: kernel_finished_session,
            },
            historical_state: historical_change_set,
            accessory: SchemaBatch::default(),
        };
        // Does not matter in this test
        let ledger_changes = SchemaBatch::default();

        // Save the change set
        storage_manager
            .save_change_set(&da_header, stf_changes, ledger_changes)
            .unwrap();

        storage_manager.finalize(&da_header).unwrap();
    }

    let (historical_root_hash, state_root_hashes) = {
        let prev_block = MockBlockHeader::from_height(blocks - 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&prev_block).unwrap();
        get_root_hashes(stf_storage)
    };

    drop(storage_manager);

    assert!(
        state_root_hashes.included_in_raw(&historical_root_hash),
        "Historical root hash {} does not contain state root hashes {} {}",
        hex::encode(historical_root_hash),
        hex::encode(state_root_hashes.user),
        hex::encode(state_root_hashes.kernel),
    );

    // Writing extra data to NOMT, both namespaces.
    // Since changes for both namespaces are always provided.
    {
        let nomt = Arc::new(NomtStateDb::<H>::new(config.clone()).unwrap());

        let the_last_block = MockBlockHeader::from_height(blocks);

        let session_builder = get_session_builder_from_committed::<H, MockHash>(nomt.clone());
        let user_session = session_builder.begin_user_session().unwrap();
        let kernel_session = session_builder.begin_kernel_session().unwrap();

        let nomt_key = KeyPath::from(the_last_block.hash.0);
        let nomt_value = Some(the_last_block.hash.0.to_vec());

        let actuals = vec![(nomt_key, nomt::KeyReadWrite::Write(nomt_value.clone()))];

        let finished_user_session = user_session.finish(actuals.clone()).unwrap();
        let finished_kernel_session = kernel_session.finish(actuals).unwrap();

        let state_finished_session =
            StateFinishedSession::new(finished_user_session, finished_kernel_session);
        nomt.commit_change_set(state_finished_session).unwrap();
    }

    let mut storage_manager =
        NomtStorageManager::<MockDaSpec, H, TestNomtStorage>::new(config.clone()).unwrap();

    // Verifying that storage is consistent after our little trick above
    let (historical_root_hash_after, state_root_hashes_after) = {
        let prev_block = MockBlockHeader::from_height(blocks - 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&prev_block).unwrap();
        get_root_hashes(stf_storage)
    };

    assert!(
        state_root_hashes_after.included_in_raw(&historical_root_hash_after),
        "Historical root hash {} does not contain state root hashes {} {}",
        hex::encode(historical_root_hash_after),
        hex::encode(state_root_hashes_after.user),
        hex::encode(state_root_hashes_after.kernel),
    );

    assert_eq!(historical_root_hash, historical_root_hash_after);
    assert_eq!(state_root_hashes.kernel, state_root_hashes_after.kernel);
    assert_eq!(state_root_hashes.user, state_root_hashes_after.user);
}

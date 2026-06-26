use std::num::{NonZeroU64, NonZeroUsize};
use std::path::Path;

use nomt::trie::KeyPath;
use rockbound::SchemaBatch;
use sha2::Digest;
use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::storage::HierarchicalStorageManager;

use super::{NomtChangeSet, NomtStorageManager, StateFinishedSession};
use crate::accessory_db::AccessoryDb;
use crate::config::{PrunerConfig, RollupDbConfig};
use crate::historical_state::HistoricalStateReader;
use crate::schema::types::slot_key::{SlotKey, SlotValue};
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

    fn materialize_from_key_values(
        self,
        items: &[(Vec<u8>, Option<Vec<u8>>)],
        version: u64,
    ) -> (Self::ChangeSet, [u8; 64]) {
        let TestNomtStorage {
            state_session_builder,
            historical_state: _,
            accessory_db: _,
        } = self;

        let user_session = state_session_builder
            .begin_user_session_without_witness()
            .unwrap();
        let kernel_session = state_session_builder
            .begin_kernel_session_without_witness()
            .unwrap();

        let mut state_writes = Vec::with_capacity(items.len());
        let mut accessory_writes = Vec::with_capacity(items.len());

        for (key, value) in items {
            let key_path = KeyPath::from(sha2::Sha256::digest(key));
            kernel_session.warm_up(key_path);
            user_session.warm_up(key_path);
            state_writes.push((key_path, nomt::KeyReadWrite::Write(value.clone())));
            accessory_writes.push((
                key.clone(),
                value.as_ref().map(|v| SlotValue::from(v.clone())),
            ));
        }

        state_writes.sort_by_key(|(k, _)| *k);

        let user_finished_session = user_session.finish(state_writes.clone()).unwrap();
        let kernel_finished_session = kernel_session.finish(state_writes.clone()).unwrap();

        let accessory_change_set =
            AccessoryDb::materialize_values(accessory_writes.clone(), SlotNumber::GENESIS).unwrap();

        let root_hash = [
            user_finished_session.root().into_inner(),
            kernel_finished_session.root().into_inner(),
        ]
        .concat();
        let historical_change_set = HistoricalStateReader::materialize_values(
            accessory_writes
                .iter()
                .map(|(k, v)| (SlotKey::from_slice(k), v.clone())),
            accessory_writes
                .iter()
                .map(|(k, v)| (SlotKey::from_slice(k), v.clone())),
            // Not used at the moment,
            root_hash.clone(),
            SlotNumber::new(version),
        )
        .unwrap();

        let root_hash = root_hash.try_into().unwrap();

        (
            NomtChangeSet {
                state: StateFinishedSession {
                    user: user_finished_session,
                    kernel: kernel_finished_session,
                },
                historical_state: historical_change_set,
                accessory: accessory_change_set,
            },
            root_hash,
        )
    }

    fn get_value(&self, key: &[u8]) -> Option<Vec<u8>> {
        let schema_key = SlotKey::from_slice(key);
        let key_path = KeyPath::from(sha2::Sha256::digest(key));
        let kernel_value = {
            let kernel_session = self
                .state_session_builder
                .begin_kernel_session_without_witness()
                .unwrap();
            kernel_session.read(key_path).unwrap()
        };
        let user_value = {
            let user_session = self
                .state_session_builder
                .begin_user_session_without_witness()
                .unwrap();
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
            .get_user_value_option_by_key(&schema_key)
            .unwrap()
            .as_ref()
            .map(|v| v.as_ref().to_vec());
        assert_eq!(historical_value_user, kernel_value);

        let historical_value_kernel = self
            .historical_state
            .get_kernel_value_option_by_key(&schema_key)
            .unwrap()
            .as_ref()
            .map(|v| v.as_ref().to_vec());
        assert_eq!(historical_value_kernel, kernel_value);

        kernel_value
    }

    fn get_value_without_consistency_checks(&self, key: &[u8]) -> Option<Vec<u8>> {
        let schema_key = SlotKey::from_slice(key);
        let historical_value_user = self
            .historical_state
            .get_user_value_option_by_key(&schema_key)
            .unwrap()
            .as_ref()
            .map(|v| v.as_ref().to_vec());

        let historical_value_kernel = self
            .historical_state
            .get_kernel_value_option_by_key(&schema_key)
            .unwrap()
            .as_ref()
            .map(|v| v.as_ref().to_vec());
        assert_eq!(historical_value_user, historical_value_kernel);

        historical_value_kernel
    }
}

type Sm = NomtStorageManager<MockDaSpec, H, TestNomtStorage>;

/// Awaits (up to 5s) any in-flight background pruner, yielding to the runtime between polls.
/// Returns immediately if no pruner is running or it has already finished.
async fn wait_for_background_pruner(storage_manager: &Sm) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match storage_manager.pruner.as_ref() {
                None => break,
                Some(pruner) if pruner.is_finished() => break,
                Some(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("pruner did not finish before timeout");
}

/// Oldest historical version still queryable after the "drain the pruner, then finalize one more
/// block" bookkeeping shared by the periodic-pruning tests. The committing pruner was spawned at
/// height `blocks - 1` and saw `last_committed = blocks - 2`, so the pruned floor sits at
/// `(blocks - 2) - versions_to_keep`.
fn oldest_available_after_drain(blocks: u64, versions_to_keep: u64) -> u64 {
    blocks - versions_to_keep - 2
}

impl TestableStorageManager for Sm {
    fn new(path: impl AsRef<Path>) -> Self {
        let config = RollupDbConfig::default_in_path(path.as_ref().to_path_buf());
        Sm::new(config, false).unwrap()
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
    removed_fork_data_view::<Sm>(true);
}

#[test]
fn test_snapshots_ordering() {
    check_snapshots_ordering::<Sm>();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ledger_finalized_height_is_updated_on_start() {
    ledger_finalized_height_is_updated_on_start::<Sm>().await;
}

/// Test the pruning behavior of the historical state. We want to check that...
///  - Queries for pruned versions return an error.
///  - Queries for unpruned versions return the correct value as of that version.
#[tokio::test(flavor = "multi_thread")]
async fn test_historical_state_with_pruning() {
    // Create a temporary directory for the test
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().to_path_buf();

    // Initialize storage manager
    let mut config = RollupDbConfig::default_in_path(db_path.clone());
    let versions_to_keep = 5;
    let pruning_frequency = 1;
    config.pruner = PrunerConfig::Periodic {
        block_interval: pruning_frequency,
        versions_to_keep: NonZeroU64::new(versions_to_keep as u64).unwrap(),
        max_batch_size: None,
    };
    let mut storage_manager =
        NomtStorageManager::<MockDaSpec, H, TestNomtStorage>::new(config.clone(), false).unwrap();

    let blocks: u64 = 14;

    // A list of the keys to write in each block.
    //  - At block 0, we write nothing.
    //  - At block 1, we write keys '1'-'10'
    //  - At block 2, we write keys '2'-'10'
    //  - etc.
    let keys_to_write = [
        vec![],
        vec![1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        vec![2, 3, 4, 5, 6, 7, 8, 9, 10],
        vec![3, 4, 5, 6, 7, 8, 9, 10],
        vec![4, 5, 6, 7, 8, 9, 10],
        vec![5, 6, 7, 8, 9, 10],
        vec![6, 7, 8, 9, 10],
        vec![7, 8, 9, 10],
        vec![8, 9, 10],
        vec![9, 10],
        vec![10],
        vec![u64::MAX], // Write a dummy value
        vec![u64::MAX], // Write a dummy value
        vec![u64::MAX], // Write a dummy value. Currently, these dummy values are needed to trigger pruning. If pruning is made to run every block, these can be removed.
    ];

    // We're just writing the keys in this loop. At each height, we set the value of each modified key to the current height.
    for height in 0u64..blocks {
        let da_header = MockBlockHeader::from_height(height + 1);
        // Create state for the block
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();

        // Materialize some test data
        let mut values = vec![];
        // For each key to write, set its value to the current height
        for key in keys_to_write[height as usize].iter() {
            let user_key = vec![*key as u8, 0, 0]; // Keys must be at least 2 bytes long, so pad with 0s.
            let value = height.to_be_bytes().to_vec();
            values.push((user_key, Some(value)));
        }
        let (stf_changes, _) = stf_storage.materialize_from_key_values(&values, height);

        // Does not matter in this test
        let ledger_changes = SchemaBatch::default();
        // Save the change set
        storage_manager
            .save_change_set(&da_header, stf_changes, ledger_changes)
            .unwrap();
        storage_manager.finalize(&da_header).unwrap();
    }

    wait_for_background_pruner(&storage_manager).await;

    // A completed pruning batch is committed only during finalization, so finalize
    // one more block in case the background pruner completed after block 14.
    let da_header = MockBlockHeader::from_height(blocks + 1);
    let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
    let dummy_key = vec![u8::MAX, 0, 0];
    let dummy_value = blocks.to_be_bytes().to_vec();
    let (stf_changes, _) =
        stf_storage.materialize_from_key_values(&[(dummy_key, Some(dummy_value))], blocks);
    storage_manager
        .save_change_set(&da_header, stf_changes, SchemaBatch::default())
        .unwrap();
    storage_manager.finalize(&da_header).unwrap();

    // Create a storage to read from after the completed pruner run has been committed.
    let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();

    // This is where the interesting logic happens.
    for key in 1..=10u64 {
        let user_key = SlotKey::from_slice(&[key as u8, 0, 0]); // Keys must be at least 2 bytes long, so pad with 0s.
                                                                // First, get the live value and assert that it's what we expect.
        let value = stf_storage
            .historical_state
            .get_user_value_option_by_key(&user_key)
            .unwrap()
            .map(|v| v.as_ref().to_vec());
        assert_eq!(value, Some(key.to_be_bytes().to_vec()));

        // The committed pruning batch was collected by the pruner spawned at height 13,
        // which saw `last_committed = blocks - 2`.
        let oldest_available_version =
            oldest_available_after_drain(blocks, versions_to_keep as u64);

        // Now, check that the value is pruned at the correct versions.
        for version in 0..keys_to_write.len() as u64 {
            let value_at_version = stf_storage
                .historical_state
                .get_user_value_option_by_key_historical(&user_key, SlotNumber::new(version));
            if version < oldest_available_version {
                // rockbound returns `Err(PrunedVersion)` for any read below
                // `oldest_available_version`, even when the underlying historical row is
                // physically still present (e.g., a single-version key whose only write
                // was preserved by the cascading delete logic).
                assert!(
                    value_at_version.is_err(),
                    "Unexpected value for key {key} at version {version}. Expected error, found {value_at_version:?}",
                );
            } else {
                let value_at_version =
                    value_at_version.expect("Query for unpruned version return error");
                // We stop writing each key at its own version. (I.e. key '1' is written in block 1, key '2' is written in blocks 1 and 2, etc.)
                let expected_value = std::cmp::min(version, key);
                assert_eq!(
                    value_at_version,
                    Some(SlotValue::from(expected_value.to_be_bytes().to_vec())),
                    "Unexpected value for key {key} at version {version}. Expected {:?}, found {value_at_version:?}",
                    expected_value.to_be_bytes().to_vec(),
                );
            }
        }
    }
}

/// `PrunerConfig::OnceAtStartup` must, when its one-time startup pass runs, synchronously prune to
/// completion (looping its internal batches until the backlog is drained) and then never spawn a
/// periodic pruner for the rest of the run. We build a prunable backlog with `OnceAtStartup`
/// configured — finalization does not prune because the periodic interval is disabled — then
/// invoke the startup prune with a tiny `max_batch_size` to force multiple internal passes.
#[tokio::test(flavor = "multi_thread")]
async fn test_prune_once_at_startup_runs_to_completion() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().to_path_buf();

    let blocks: u64 = 14;
    // Key `k` (1..=10) is written at versions 1..=k with value == version; later blocks write a
    // dummy key. Same workload as `test_historical_state_with_pruning`.
    let keys_to_write = [
        vec![],
        vec![1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        vec![2, 3, 4, 5, 6, 7, 8, 9, 10],
        vec![3, 4, 5, 6, 7, 8, 9, 10],
        vec![4, 5, 6, 7, 8, 9, 10],
        vec![5, 6, 7, 8, 9, 10],
        vec![6, 7, 8, 9, 10],
        vec![7, 8, 9, 10],
        vec![8, 9, 10],
        vec![9, 10],
        vec![10],
        vec![u64::MAX],
        vec![u64::MAX],
        vec![u64::MAX],
    ];

    let versions_to_keep = 5u64;
    let mut config = RollupDbConfig::default_in_path(db_path);
    config.pruner = PrunerConfig::OnceAtStartup {
        versions_to_keep: NonZeroU64::new(versions_to_keep).unwrap(),
        // Tiny batch: the backlog far exceeds it, forcing several internal commit passes.
        max_batch_size: Some(NonZeroUsize::new(8).unwrap()),
        compact_after: false,
    };
    let mut storage_manager =
        NomtStorageManager::<MockDaSpec, H, TestNomtStorage>::new(config, false).unwrap();

    // Build a prunable backlog. `OnceAtStartup` leaves periodic pruning disabled (the finalize
    // interval is `None`), so finalization never prunes — the backlog accumulates until the
    // explicit startup prune below.
    for height in 0u64..blocks {
        let da_header = MockBlockHeader::from_height(height + 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
        let mut values = vec![];
        for key in keys_to_write[height as usize].iter() {
            let user_key = vec![*key as u8, 0, 0]; // Keys must be at least 2 bytes long.
            values.push((user_key, Some(height.to_be_bytes().to_vec())));
        }
        let (stf_changes, _) = stf_storage.materialize_from_key_values(&values, height);
        storage_manager
            .save_change_set(&da_header, stf_changes, SchemaBatch::default())
            .unwrap();
        storage_manager.finalize(&da_header).unwrap();
    }

    // Finalization did not prune (periodic interval disabled), and no pruner is in flight.
    assert_eq!(storage_manager.pruning_commits_count(), 0);
    assert!(
        !storage_manager.is_pruner_running(),
        "no pruner should be in flight before the startup prune"
    );

    // Run the one-time startup prune.
    storage_manager.prune_once_at_startup().unwrap();

    // The tiny batch forces multiple commit passes (proving prune-to-completion), the run is
    // synchronous (nothing left in flight), and the final pass drained the backlog.
    let commits_after_startup = storage_manager.pruning_commits_count();
    assert!(
        commits_after_startup >= 2,
        "expected several startup-prune passes, got {commits_after_startup}"
    );
    assert!(
        !storage_manager.is_pruner_running(),
        "startup prune must be synchronous (no background pruner left running)"
    );
    assert!(
        !storage_manager.last_pruning_hit_size_limit(),
        "the final startup-prune pass should have fully drained the backlog"
    );

    // The startup prune had real effect: the oldest version is now pruned, while live values
    // survive.
    let (stf_storage, _ledger_storage) = storage_manager
        .create_state_after(&MockBlockHeader::from_height(blocks))
        .unwrap();
    for key in 1..=10u64 {
        let user_key = SlotKey::from_slice(&[key as u8, 0, 0]);
        let live_value = stf_storage
            .historical_state
            .get_user_value_option_by_key(&user_key)
            .unwrap()
            .map(|v| v.as_ref().to_vec());
        assert_eq!(
            live_value,
            Some(key.to_be_bytes().to_vec()),
            "live value for key {key} should survive pruning"
        );

        let oldest = stf_storage
            .historical_state
            .get_user_value_option_by_key_historical(&user_key, SlotNumber::new(0));
        assert!(
            oldest.is_err(),
            "version 0 should be pruned for key {key}, found {oldest:?}"
        );
    }

    // --- Phase 3: subsequent finalizations must NOT spawn a periodic pruner. ---
    let commits_before = storage_manager.pruning_commits_count();
    for height in blocks..(blocks + 3) {
        let da_header = MockBlockHeader::from_height(height + 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
        let (stf_changes, _) = stf_storage.materialize_from_key_values(
            &[(vec![u8::MAX, 0, 0], Some(height.to_be_bytes().to_vec()))],
            height,
        );
        storage_manager
            .save_change_set(&da_header, stf_changes, SchemaBatch::default())
            .unwrap();
        storage_manager.finalize(&da_header).unwrap();
        assert!(
            !storage_manager.is_pruner_running(),
            "OnceAtStartup must not spawn a periodic pruner during finalization"
        );
    }
    assert_eq!(
        storage_manager.pruning_commits_count(),
        commits_before,
        "OnceAtStartup must not prune again after the one-time startup pass"
    );
}

/// `OnceAtStartup` with `compact_after: true` must run the post-prune compaction of every pruned
/// column family — accessory plus the user/kernel archival historical + pruning CFs (via
/// `DbGroup::compact_pruned_cfs` → `VersionedDB::trigger_compaction`) — without error, and reads
/// must stay correct afterward. This guards the user/kernel compaction wiring added with the
/// rockbound rev bump; `test_prune_once_at_startup_runs_to_completion` covers the same path with
/// compaction off.
#[tokio::test(flavor = "multi_thread")]
async fn test_prune_once_at_startup_compacts_pruned_cfs() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().to_path_buf();

    // Key `k` (1..=6) is written at versions 1..=k with value == version, building a multi-version
    // backlog so the startup prune has tombstones to compact away.
    let blocks: u64 = 7;
    let keys_to_write = [
        vec![],
        vec![1u64, 2, 3, 4, 5, 6],
        vec![2, 3, 4, 5, 6],
        vec![3, 4, 5, 6],
        vec![4, 5, 6],
        vec![5, 6],
        vec![6],
    ];

    let versions_to_keep = 2u64;
    let mut config = RollupDbConfig::default_in_path(db_path);
    config.pruner = PrunerConfig::OnceAtStartup {
        versions_to_keep: NonZeroU64::new(versions_to_keep).unwrap(),
        max_batch_size: None,
        compact_after: true,
    };
    let mut storage_manager =
        NomtStorageManager::<MockDaSpec, H, TestNomtStorage>::new(config, false).unwrap();

    for height in 0u64..blocks {
        let da_header = MockBlockHeader::from_height(height + 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
        let mut values = vec![];
        for key in keys_to_write[height as usize].iter() {
            let user_key = vec![*key as u8, 0, 0]; // Keys must be at least 2 bytes long.
            values.push((user_key, Some(height.to_be_bytes().to_vec())));
        }
        let (stf_changes, _) = stf_storage.materialize_from_key_values(&values, height);
        storage_manager
            .save_change_set(&da_header, stf_changes, SchemaBatch::default())
            .unwrap();
        storage_manager.finalize(&da_header).unwrap();
    }

    // Prune to completion AND compact the pruned column families. The compaction step (accessory +
    // user/kernel `trigger_compaction`) must not panic or error.
    storage_manager.prune_once_at_startup().unwrap();

    // Reads remain correct after compaction: live values survive, and the oldest version is pruned.
    let (stf_storage, _ledger_storage) = storage_manager
        .create_state_after(&MockBlockHeader::from_height(blocks))
        .unwrap();
    for key in 1..=6u64 {
        let user_key = SlotKey::from_slice(&[key as u8, 0, 0]);
        let live_value = stf_storage
            .historical_state
            .get_user_value_option_by_key(&user_key)
            .unwrap()
            .map(|v| v.as_ref().to_vec());
        assert_eq!(
            live_value,
            Some(key.to_be_bytes().to_vec()),
            "live value for key {key} should survive prune + compaction"
        );

        let oldest = stf_storage
            .historical_state
            .get_user_value_option_by_key_historical(&user_key, SlotNumber::new(0));
        assert!(
            oldest.is_err(),
            "version 0 should be pruned for key {key} after compaction, found {oldest:?}"
        );
    }
}

/// Hot-key workload (the case #3018 cares about): write the SAME key in every block, far more
/// times than `versions_to_keep`. After pruning, only the recent window must remain queryable —
/// a hot key must not accumulate unbounded historical versions.
///
/// This mirrors the block/drain bookkeeping of `test_historical_state_with_pruning` exactly (same
/// `blocks`, `versions_to_keep`, drain + one extra finalize), so the same
/// `oldest_available_version = blocks - versions_to_keep - 2` boundary applies.
#[tokio::test(flavor = "multi_thread")]
async fn test_hot_key_pruning_keeps_only_recent_versions() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().to_path_buf();

    let mut config = RollupDbConfig::default_in_path(db_path);
    let versions_to_keep = 5usize;
    config.pruner = PrunerConfig::Periodic {
        block_interval: 1,
        versions_to_keep: NonZeroU64::new(versions_to_keep as u64).unwrap(),
        max_batch_size: None,
    };
    let mut storage_manager =
        NomtStorageManager::<MockDaSpec, H, TestNomtStorage>::new(config, false).unwrap();

    let blocks: u64 = 14;
    let hot_key_bytes = vec![42u8, 0, 0];

    // Write the single hot key in every block (block 0 writes nothing, like the sibling test).
    // Value is multi-byte and asymmetric (`0x1000 + height`) to catch endianness bugs.
    for height in 0u64..blocks {
        let da_header = MockBlockHeader::from_height(height + 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
        let values: Vec<(Vec<u8>, Option<Vec<u8>>)> = if height == 0 {
            vec![]
        } else {
            vec![(
                hot_key_bytes.clone(),
                Some((0x1000u64 + height).to_be_bytes().to_vec()),
            )]
        };
        let (stf_changes, _) = stf_storage.materialize_from_key_values(&values, height);
        storage_manager
            .save_change_set(&da_header, stf_changes, SchemaBatch::default())
            .unwrap();
        storage_manager.finalize(&da_header).unwrap();
    }

    // Wait for the background pruner to finish.
    wait_for_background_pruner(&storage_manager).await;

    // A completed pruning batch is committed only during finalization; finalize one more block
    // (writing the hot key again) so the batch lands and the live value is at version `blocks`.
    let da_header = MockBlockHeader::from_height(blocks + 1);
    let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
    let (stf_changes, _) = stf_storage.materialize_from_key_values(
        &[(
            hot_key_bytes.clone(),
            Some((0x1000u64 + blocks).to_be_bytes().to_vec()),
        )],
        blocks,
    );
    storage_manager
        .save_change_set(&da_header, stf_changes, SchemaBatch::default())
        .unwrap();
    storage_manager.finalize(&da_header).unwrap();

    // Read after the committed pruner run.
    let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
    let hot_key = SlotKey::from_slice(&hot_key_bytes);

    // The live (latest) value is the most recent write and must always be readable.
    let live = stf_storage
        .historical_state
        .get_user_value_option_by_key(&hot_key)
        .unwrap()
        .map(|v| v.as_ref().to_vec());
    assert_eq!(
        live,
        Some((0x1000u64 + blocks).to_be_bytes().to_vec()),
        "Hot key live value should be the most recent write"
    );

    // Same bookkeeping as `test_historical_state_with_pruning`: the committing pruner saw
    // `last_committed = blocks - 2`, so the oldest retained version is `last_committed - keep`.
    let oldest_available_version = oldest_available_after_drain(blocks, versions_to_keep as u64);

    let mut retained = 0u64;
    for version in 0..blocks {
        let value_at_version = stf_storage
            .historical_state
            .get_user_value_option_by_key_historical(&hot_key, SlotNumber::new(version));
        if version < oldest_available_version {
            assert!(
                value_at_version.is_err(),
                "Hot key at pruned version {version} should error, found {value_at_version:?}",
            );
        } else {
            let value = value_at_version.expect("Query for unpruned version returned error");
            // The hot key is written every block, so its value as of `version` is `0x1000 + version`.
            assert_eq!(
                value,
                Some(SlotValue::from(
                    (0x1000u64 + version).to_be_bytes().to_vec()
                )),
                "Hot key value mismatch at version {version}",
            );
            retained += 1;
        }
    }

    // Only the post-prune window survives: the hot key did NOT accumulate all `blocks` versions.
    assert_eq!(
        retained,
        blocks - oldest_available_version,
        "Hot key should retain only the recent window [oldest_available_version, blocks)",
    );
}

/// Pruner backpressure: with a tiny `max_batch_size`, a large prunable backlog cannot be
/// cleared in one pass. The pruner must report `hit_size_limit = true` and the storage manager
/// must re-spawn it across subsequent finalizations until the backlog is drained (the loop in
/// `finalize`). We observe this via the test-only hooks on `NomtStorageManager`.
#[tokio::test(flavor = "multi_thread")]
async fn test_pruner_backpressure_respawns_until_drained() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().to_path_buf();

    let mut config = RollupDbConfig::default_in_path(db_path);
    let versions_to_keep = 2usize;
    // Tiny batch: a single block of overwrites already exceeds this, forcing multiple passes.
    config.pruner = PrunerConfig::Periodic {
        block_interval: 1,
        versions_to_keep: NonZeroU64::new(versions_to_keep as u64).unwrap(),
        max_batch_size: Some(NonZeroUsize::new(8).unwrap()),
    };
    let mut storage_manager =
        NomtStorageManager::<MockDaSpec, H, TestNomtStorage>::new(config, false).unwrap();

    let num_keys = 20usize;
    let initial_blocks: u64 = 5;

    // Overwrite many keys every block so prunable historical rows (>> max_batch_size) pile up.
    for height in 0u64..initial_blocks {
        let da_header = MockBlockHeader::from_height(height + 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
        let mut values: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::with_capacity(num_keys);
        for k in 0..num_keys {
            let key = vec![(k & 0xff) as u8, ((k >> 8) & 0xff) as u8, 0];
            values.push((key, Some((0x1000u64 + height).to_be_bytes().to_vec())));
        }
        let (stf_changes, _) = stf_storage.materialize_from_key_values(&values, height);
        storage_manager
            .save_change_set(&da_header, stf_changes, SchemaBatch::default())
            .unwrap();
        storage_manager.finalize(&da_header).unwrap();
    }

    // Drive finalizations, draining the pruner between each so its batch commits on the next
    // finalize. A dedicated "tick" key (outside the 0..num_keys range) advances the chain.
    let tick_key = vec![0xAAu8, 0xAA, 0];
    let mut saw_hit_limit = storage_manager.last_pruning_hit_size_limit();
    let drain_iterations = 30u64;
    for i in 0..drain_iterations {
        storage_manager.wait_for_pruner_to_finish();
        let height = initial_blocks + i;
        let da_header = MockBlockHeader::from_height(height + 1);
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
        let (stf_changes, _) = stf_storage.materialize_from_key_values(
            &[(
                tick_key.clone(),
                Some((0x2000u64 + height).to_be_bytes().to_vec()),
            )],
            height,
        );
        storage_manager
            .save_change_set(&da_header, stf_changes, SchemaBatch::default())
            .unwrap();
        storage_manager.finalize(&da_header).unwrap();
        saw_hit_limit |= storage_manager.last_pruning_hit_size_limit();
    }

    assert!(
        saw_hit_limit,
        "pruner should have hit the batch size limit at least once with max_batch_size = 8",
    );
    assert!(
        storage_manager.pruning_commits_count() >= 2,
        "pruner should have committed multiple batches (backpressure respawn loop), got {}",
        storage_manager.pruning_commits_count(),
    );

    // Backpressure must eventually DRAIN, not stall: after many passes the pruned floor has
    // advanced well past the early versions, so old versions of an original key are gone while
    // its carried-forward live value remains readable.
    let da_header = MockBlockHeader::from_height(initial_blocks + drain_iterations + 1);
    let (stf_storage, _ledger_storage) = storage_manager.create_state_for(&da_header).unwrap();
    let sample_key = SlotKey::from_slice(&[0u8, 0, 0]);
    let live = stf_storage
        .historical_state
        .get_user_value_option_by_key(&sample_key)
        .unwrap();
    assert!(
        live.is_some(),
        "sampled key should still have a live (carried-forward) value after pruning",
    );
    let oldest = stf_storage
        .historical_state
        .get_user_value_option_by_key_historical(&sample_key, SlotNumber::new(0));
    assert!(
        oldest.is_err(),
        "version 0 of the sampled key should be pruned once backpressure has drained, found {oldest:?}",
    );
}

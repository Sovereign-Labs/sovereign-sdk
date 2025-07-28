#![allow(missing_docs)]
use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::ledger_api::LedgerStateProvider;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use tokio::time::Instant;

use super::data_helpers::{
    get_expected_chain_values, materialize_ledger_changes, verify_ledger_storage,
};
use crate::ledger_db::LedgerDb;
use crate::schema::types::{BatchNumber, StoredSlot};
use crate::storage_manager::tests::arbitrary::{get_block_hash, ForkDescription, ForkMap};

pub trait TestableStorage: Sized {
    type ChangeSet;

    /// Build [`Self::ChangeSet`] that contains data related to given [`MockBlockHeader`].
    /// Data is the following: block_height => block_hash.
    fn materialize_from_block(self, da_header: &MockBlockHeader) -> Self::ChangeSet {
        let height = da_header.height().to_be_bytes().to_vec();
        let hash_bytes = da_header.hash().0.to_vec();
        self.materialize_from_key_value(height, Some(hash_bytes))
    }

    fn materialize_from_key_value(self, key: Vec<u8>, value: Option<Vec<u8>>) -> Self::ChangeSet {
        let items = [(key, value)];
        self.materialize_from_key_values(&items)
    }
    fn materialize_from_key_values(self, items: &[(Vec<u8>, Option<Vec<u8>>)]) -> Self::ChangeSet;
    fn get_value(&self, key: &[u8]) -> Option<Vec<u8>>;
}

pub trait TestableStorageManager:
    HierarchicalStorageManager<MockDaSpec, LedgerState = DeltaReader, LedgerChangeSet = SchemaBatch>
    + Send
    + Sync
where
    Self::StfState: TestableStorage<ChangeSet = Self::StfChangeSet>,
{
    fn new(path: impl AsRef<std::path::Path>) -> Self;
    fn verify_stf_storage(stf_storage: &Self::StfState, expected_values: &[(u64, MockHash)]);
    fn is_empty(&self) -> bool;
    fn snapshots_count(&self) -> usize;
    fn blocks_to_parent_count(&self) -> usize;
}

// Checking the typical lifecycle of the storage in linear progression of the chain,
// meaning no forks happen, and DA height progresses incrementally by 1.
// At each height:
// 1. Bootstrap storage is created and validated.
// 2. Storage for this block is created.
// 3. Changes for this block are materialized and saved.
// 4. If finalization needs to happen for some block, finalization happens.
pub fn linear_progression<Sm: TestableStorageManager>(to_height: u64, finality: u64)
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());
    // Chain starts from height 1
    let mut chain = Vec::with_capacity(to_height as usize);

    for height in 1..=to_height {
        // Bootstrap storage, check that finalized data has been written to the disk
        {
            let finalized_index = chain.len().saturating_sub(finality as usize);
            let prev_header =
                MockBlockHeader::from_height(finalized_index.saturating_sub(1) as u64);
            let expected_finalized_values = get_expected_chain_values(&chain[..finalized_index]);

            let (bootstrap_stf_storage, bootstrap_ledger_storage) =
                storage_manager.create_state_after(&prev_header).unwrap();

            Sm::verify_stf_storage(&bootstrap_stf_storage, &expected_finalized_values[..]);
            verify_ledger_storage(&bootstrap_ledger_storage, &expected_finalized_values[..]);
        }

        let da_header = MockBlockHeader::from_height(height);
        // Regular storage for stf
        {
            let expected_values = get_expected_chain_values(&chain[..]);
            let (stf_storage, ledger_storage) =
                storage_manager.create_state_for(&da_header).unwrap();
            Sm::verify_stf_storage(&stf_storage, &expected_values);
            verify_ledger_storage(&ledger_storage, &expected_values[..]);
            let stf_changes = stf_storage.materialize_from_block(&da_header);
            let ledger_changes = materialize_ledger_changes(&da_header);
            storage_manager
                .save_change_set(&da_header, stf_changes, ledger_changes)
                .unwrap();
        }
        chain.push(da_header.clone());

        // API Storage
        {
            let (stf_storage, ledger_storage) =
                storage_manager.create_state_after(&da_header).unwrap();
            let expected_values = get_expected_chain_values(&chain[..]);
            Sm::verify_stf_storage(&stf_storage, &expected_values);
            verify_ledger_storage(&ledger_storage, &expected_values[..]);
        }

        if let Some(final_height) = height.checked_sub(finality) {
            if final_height > 0 {
                let final_header = MockBlockHeader::from_height(final_height);
                storage_manager.finalize(&final_header).unwrap();
            }
        }
    }

    let should_be_empty = finality == 0;
    assert_eq!(should_be_empty, storage_manager.is_empty());
}

pub enum ExplorationMode {
    // Breath first.
    Bfs,
    // Depth first.
    Dfs,
}

impl ExplorationMode {
    fn get_next_hash(&self, queue: &mut VecDeque<MockHash>) -> Option<MockHash> {
        match self {
            ExplorationMode::Bfs => queue.pop_front(),
            ExplorationMode::Dfs => queue.pop_back(),
        }
    }
}

// Create and save changes from all forks, iterating by height
pub fn test_exploration<Sm: TestableStorageManager>(
    fork: ForkDescription,
    exploration_mode: ExplorationMode,
) where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    let fork_map = ForkMap::from(fork);
    let start = fork_map.get_start().expect("Empty chain-map");
    let mut next_blocks = VecDeque::new();
    next_blocks.push_back(start);

    let mut highest_block = MockBlockHeader::from_height(0);

    while let Some(block_hash) = exploration_mode.get_next_hash(&mut next_blocks) {
        for child in fork_map.get_child_hashes(&block_hash) {
            next_blocks.push_back(child);
        }

        let da_header = fork_map.get_block_header(&block_hash).unwrap();
        if da_header.height() > highest_block.height() {
            highest_block = da_header.clone();
        }

        let (stf_storage, ledger_storage) = storage_manager
            .create_state_for(da_header)
            .expect("Creating storage failed");

        let this_chain = fork_map.get_chain_up_to(da_header.clone());
        let expected_values = get_expected_chain_values(&this_chain[..this_chain.len() - 1]);
        Sm::verify_stf_storage(&stf_storage, &expected_values[..]);
        verify_ledger_storage(&ledger_storage, &expected_values[..]);

        let stf_changes = stf_storage.materialize_from_block(da_header);
        let ledger_changes = materialize_ledger_changes(da_header);

        storage_manager
            .save_change_set(da_header, stf_changes, ledger_changes)
            .expect("Saving change set has failed");
        assert_eq!(
            storage_manager.snapshots_count(),
            storage_manager.blocks_to_parent_count(),
        );

        let (stf_storage, ledger_storage) = storage_manager.create_state_after(da_header).unwrap();
        assert_eq!(
            storage_manager.snapshots_count(),
            storage_manager.blocks_to_parent_count(),
        );
        let expected_values = get_expected_chain_values(&this_chain[..this_chain.len()]);
        Sm::verify_stf_storage(&stf_storage, &expected_values[..]);
        verify_ledger_storage(&ledger_storage, &expected_values[..]);
    }
    // Finalizing longest chain
    let longest_chain = fork_map.get_chain_up_to(highest_block);
    for block_header in longest_chain {
        storage_manager.finalize(&block_header).unwrap();
    }
    // State manager should be empty after the longest chain is finalized.
    assert!(storage_manager.is_empty());
}

pub fn minimal_fork_bfs<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
{
    let fork = ForkDescription {
        start_height: 1,
        length: 3,
        child_forks: vec![
            ForkDescription {
                start_height: 1,
                length: 5,
                child_forks: Vec::new(),
            },
            ForkDescription {
                start_height: 1,
                length: 5,
                child_forks: Vec::new(),
            },
        ],
    };
    test_exploration::<Sm>(fork.clone(), ExplorationMode::Bfs);
    test_exploration::<Sm>(fork, ExplorationMode::Dfs);
}

/// Checks calls on empty storage return a proper error
pub fn calls_on_empty<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    assert!(storage_manager.is_empty());

    let da_header = MockBlockHeader::from_height(1);
    let save_result =
        storage_manager.save_change_set(&da_header, Default::default(), SchemaBatch::default());
    assert!(save_result.is_err());
    // storage_manager.dbg_validate_internal_consistency();
    let expected_msg = format!(
        "Attempt to save changeset for unknown block header {}",
        da_header.display()
    );
    assert_eq!(expected_msg, save_result.unwrap_err().to_string());

    let finalize_result = storage_manager.finalize(&da_header);
    assert!(finalize_result.is_err());
    let expected_msg = format!(
        "No changes has been previously saved for block header prev_hash={} next_hash={}",
        da_header.prev_hash, da_header.hash,
    );
    assert_eq!(expected_msg, finalize_result.unwrap_err().to_string());
    assert!(storage_manager.is_empty());
}

/// Checks that calling creates storage multiple times for the same block works without errors
pub fn double_create_storage<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    let genesis_header = MockBlockHeader::from_height(0);
    let da_header = MockBlockHeader::from_height(1);
    // Bootstrap storage
    let _ = storage_manager.create_state_after(&genesis_header).unwrap();
    let _ = storage_manager.create_state_after(&genesis_header).unwrap();

    // Normal storage
    let (stf_storage, _) = storage_manager.create_state_for(&da_header).unwrap();
    let _ = storage_manager.create_state_for(&da_header).unwrap();

    let stf_changes = stf_storage.materialize_from_block(&da_header);
    let ledger_changes = materialize_ledger_changes(&da_header);
    storage_manager
        .save_change_set(&da_header, stf_changes, ledger_changes)
        .unwrap();

    // After block
    let _ = storage_manager.create_state_after(&da_header).unwrap();
    let _ = storage_manager.create_state_after(&da_header).unwrap();
}

fn attempt_to_save_unknown_block<Sm: TestableStorageManager>(
    storage_manager: &mut Sm,
    da_header: &MockBlockHeader,
) where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let result =
        storage_manager.save_change_set(da_header, Default::default(), SchemaBatch::default());
    assert!(result.is_err());
    let expected_error_message = format!(
        "Attempt to save changeset for unknown block header {}",
        da_header.display()
    );
    assert_eq!(expected_error_message, result.unwrap_err().to_string());
}

/// Checks that block that hasn't been seen by a storage manager via creating storage,
/// won't be accepted to save.
pub fn unknown_block_cannot_be_saved<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    // On an empty map
    let da_header_1 = MockBlockHeader::from_height(1);
    attempt_to_save_unknown_block(&mut storage_manager, &da_header_1);
    let (stf_storage, _) = storage_manager.create_state_for(&da_header_1).unwrap();
    let stf_changes = stf_storage.materialize_from_block(&da_header_1);
    let ledger_changes = materialize_ledger_changes(&da_header_1);
    storage_manager
        .save_change_set(&da_header_1, stf_changes, ledger_changes)
        .unwrap();

    // On something
    let da_header_2 = MockBlockHeader::from_height(2);
    attempt_to_save_unknown_block(&mut storage_manager, &da_header_2);
}

pub fn double_save_changes<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());
    let da_header = MockBlockHeader::from_height(1);
    let _ = storage_manager.create_state_for(&da_header).unwrap();
    // the first save everything is good.
    storage_manager
        .save_change_set(&da_header, Default::default(), SchemaBatch::new())
        .unwrap();

    // the second save should fail, as it indicates a bug.
    let result =
        storage_manager.save_change_set(&da_header, Default::default(), SchemaBatch::default());
    assert!(result.is_err());
    let expected_message = format!(
        "Attempt to save changes for the same block {} twice. Probably a bug.",
        da_header.display()
    );
    assert_eq!(expected_message, result.unwrap_err().to_string());
}

/// Demonstrates conditions for a storage manager to create storage "after" the block.
pub fn create_state_after_not_saved_block<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState: TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>
        + Debug,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());
    let da_header = MockBlockHeader::from_height(1);

    let _ = storage_manager.create_state_for(&da_header).unwrap();

    let (stf_storage_after, ledger_after) = storage_manager.create_state_after(&da_header).unwrap();
    Sm::verify_stf_storage(&stf_storage_after, &[]);
    verify_ledger_storage(&ledger_after, &[]);

    let stf_changes = stf_storage_after.materialize_from_block(&da_header);
    // It still cannot see values, after it has changed.
    let (stf_storage_after, ledger_after) = storage_manager.create_state_after(&da_header).unwrap();
    let ledger_changes = materialize_ledger_changes(&da_header);
    storage_manager
        .save_change_set(&da_header, stf_changes, ledger_changes)
        .unwrap();

    // It still cannot see values, after it has changed.
    Sm::verify_stf_storage(&stf_storage_after, &[]);
    verify_ledger_storage(&ledger_after, &[]);
    // Dropping so finalization can happen.
    drop(stf_storage_after);
    storage_manager.finalize(&da_header).unwrap();
    let (stf_storage_after, ledger_after) = storage_manager.create_state_after(&da_header).unwrap();
    let some_data = vec![(da_header.height(), da_header.hash())];
    Sm::verify_stf_storage(&stf_storage_after, &some_data);
    verify_ledger_storage(&ledger_after, &some_data);
}

pub fn finalize_only_last_block<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());
    let to_height = 5;
    for height in 1..=to_height {
        let da_header = MockBlockHeader::from_height(height);
        let (stf_storage, _) = storage_manager.create_state_for(&da_header).unwrap();
        let stf_changes = stf_storage.materialize_from_block(&da_header);
        let ledger_changes = materialize_ledger_changes(&da_header);
        storage_manager
            .save_change_set(&da_header, stf_changes, ledger_changes)
            .unwrap();
    }
    let da_header = MockBlockHeader::from_height(to_height);
    storage_manager.finalize(&da_header).unwrap();
    assert!(&storage_manager.is_empty());
}
/// Blocks relation is the following:
/// 1 -> 2 -> ... -> n-1 -> n
///                  / -> E
/// A -> B -> ... -> C -> D
///                  \ -> F
///                      ...
///                   ... X
/// E, H, G, etc. are moved to a separate thread.
/// They read data from each snapshot all the time,
/// checking that data from each for is present.
/// Validation:
/// First test measures how much time on average it takes
/// to do such validation single threaded without any concurrent readings
/// Then it starts X threads for each "fork" to do the same validation.
/// Each thread does 2 iterations of reading:
/// just concurrent reading and then concurrent reading while blocks are finalized.
/// Then test checks
/// that avg time each thread spent on these is not more than 3 times a single reading.
pub fn parallel_forks_reading_while_finalization_is_happening<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState: TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>
        + 'static
        + Send,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    // this is X.
    // So the total value of concurrent threads will be 8,
    // which is a comfortable choice for many machines.
    let sub_forks_count = 7;
    // this is n
    // Enough blocks will be written on disk during the finalization phase.
    let main_fork_len = 30;
    let sub_fork_start = (main_fork_len - 1) as u64;
    let fork_description = ForkDescription {
        start_height: 1,
        length: main_fork_len,
        child_forks: vec![
            ForkDescription {
                start_height: sub_fork_start,
                length: 1,
                child_forks: Vec::new(),
            };
            sub_forks_count
        ],
    };

    let fork_map = ForkMap::from(fork_description);
    assert_eq!(
        sub_forks_count + main_fork_len as usize,
        fork_map.blocks_count()
    );

    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    // fill storage manager
    let start = fork_map.get_start().expect("Empty chain-map");
    let mut next_blocks = VecDeque::new();
    next_blocks.push_back(start);
    while let Some(block_hash) = next_blocks.pop_front() {
        for child in fork_map.get_child_hashes(&block_hash) {
            next_blocks.push_back(child);
        }
        let da_header = fork_map.get_block_header(&block_hash).unwrap();
        let (stf_storage, _) = storage_manager
            .create_state_for(da_header)
            .expect("Creating storage failed");
        let stf_changes = stf_storage.materialize_from_block(da_header);
        let ledger_changes = materialize_ledger_changes(da_header);
        storage_manager
            .save_change_set(da_header, stf_changes, ledger_changes)
            .expect("Saving change set has failed");
    }

    let mut prepare_for_reading = |fork_id: u64| {
        let block_hash = get_block_hash(fork_id, main_fork_len as u64);
        let block_header = fork_map.get_block_header(&block_hash).unwrap();
        let this_chain = fork_map.get_chain_up_to(block_header.clone());
        let expected_values = get_expected_chain_values(&this_chain[..this_chain.len()]);
        let (stf_storage, ledger_storage) = storage_manager
            .create_state_after(block_header)
            .expect("Creating storage failed");

        (stf_storage, ledger_storage, expected_values)
    };

    let reading_count = 1000;

    let record_reading =
        move |stf: &Sm::StfState, ledger: &DeltaReader, expected: &[(u64, MockHash)]| -> Duration {
            let mut spent_reading = Duration::default();
            for _ in 0..reading_count {
                let start_validation = Instant::now();
                Sm::verify_stf_storage(stf, expected);
                verify_ledger_storage(ledger, expected);
                spent_reading += start_validation.elapsed();
            }
            spent_reading / reading_count
        };

    // Record how much it takes to do a round of reading from the main fork without any concurrency.
    let average_reading_time_single_access = {
        let (stf_storage, ledger_storage, expected_values) = prepare_for_reading(1);
        record_reading(&stf_storage, &ledger_storage, &expected_values[..])
    };

    let avg_reading_time_threshold = average_reading_time_single_access.checked_mul(3).unwrap();

    let total_forks = sub_forks_count + 1; // main fork
    let barrier = Arc::new(Barrier::new(total_forks));

    let mut handles = vec![];
    // Starting fork readers
    for fork_id in 1..total_forks {
        let (stf_storage, ledger_storage, expected_values) = prepare_for_reading(fork_id as u64);

        let barrier = Arc::clone(&barrier);

        // Each fork counts how many reads it completed.
        handles.push(std::thread::spawn(move || -> (Duration, Duration) {
            // First, we record how much time each thread took reading concurrently;
            let spent_reading_concurrently =
                record_reading(&stf_storage, &ledger_storage, &expected_values[..]);

            // Then we wait for finalization to start
            barrier.wait();

            let spent_reading_during_finalization =
                record_reading(&stf_storage, &ledger_storage, &expected_values[..]);
            (
                spent_reading_concurrently,
                spent_reading_during_finalization,
            )
        }));
    }

    barrier.wait();
    let mut finalization_duration = Duration::default();
    for height in 1..main_fork_len {
        let start = Instant::now();
        let block_hash = get_block_hash(1, height as u64);
        let block_header = fork_map.get_block_header(&block_hash).unwrap();
        storage_manager.finalize(block_header).unwrap();
        finalization_duration += start.elapsed();
    }
    for handle in handles {
        let (spent_reading_concurrently, spent_reading_finalization) =
            handle.join().expect("Thread panicked");
        assert!(
            spent_reading_concurrently < avg_reading_time_threshold,
            "Concurrent reading {spent_reading_concurrently:?} is worse than max allowed {avg_reading_time_threshold:?}"
        );
        assert!(
            spent_reading_finalization < avg_reading_time_threshold,
            "Concurrent reading during finalization {spent_reading_finalization:?} is worse than max allowed {avg_reading_time_threshold:?}"
        );
    }
}

/// At each height there happens x forks.
/// They all create storage for themselves.
/// Then they all save some changes.
/// Then 1 is finalized after x blocks.
/// The purpose of this is to check that at a given height,
/// several storages can be created without saving all the data.
pub fn several_jumping_forks<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState:
        TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let to_height = 10;
    let finality = 3;
    let forks_number = 5;
    let main_fork_id = 1;

    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    let mut fork_headers = Vec::with_capacity(forks_number as usize);
    for height in 1..=to_height {
        fork_headers.clear();
        for fork_id in 1..=forks_number {
            let prev_hash = get_block_hash(fork_id, height - 1);
            let hash = get_block_hash(fork_id, height);
            fork_headers.push(MockBlockHeader {
                prev_hash,
                hash,
                height,
                ..Default::default()
            });
        }
        // Create storage for each fork.
        for da_header in &fork_headers {
            let _ = storage_manager.create_state_for(da_header).unwrap();
        }
        // Save storage for each fork.
        for da_header in &fork_headers {
            storage_manager
                .save_change_set(da_header, Default::default(), SchemaBatch::default())
                .unwrap();
        }

        if let Some(final_height) = height.checked_sub(finality) {
            if final_height > 0 {
                let prev_hash = get_block_hash(main_fork_id, final_height - 1);
                let hash = get_block_hash(main_fork_id, final_height);
                let final_header = MockBlockHeader {
                    prev_hash,
                    hash,
                    height: final_height,
                    ..Default::default()
                };
                storage_manager.finalize(&final_header).unwrap();
            }
        }
    }
}

// 2 helper functions for the following tests.
// Here is a chain schema:
//  A -> B
//   \-> C
// Returns A, B, C
pub(crate) fn get_parent_and_2_children() -> (MockBlockHeader, MockBlockHeader, MockBlockHeader) {
    let block_a = MockBlockHeader::from_height(1);
    // Changes are in rocksdb now, creating readers
    let block_b = MockBlockHeader {
        prev_hash: block_a.hash(),
        hash: get_block_hash(1, 2),
        height: 2,
        ..Default::default()
    };
    let block_c = MockBlockHeader {
        prev_hash: block_a.hash(),
        hash: get_block_hash(2, 2),
        height: 2,
        ..Default::default()
    };
    (block_a, block_b, block_c)
}

// "Orphaned fork" is a fork appears when finalization happens on different fork past the start height of this fork.
// Meaning that this fork should be discarded completely, because data from the sibling fork was finalized.
// This test documents behavior that observed from this orphaned fork.
// This might be useful to know if there's a long-running task that relies on data from a fork that has been orphaned.
// Test details.
// Here is the chain schema:
//  A -> B
//   \-> C
// Block A has key=1 value=1.
// This block is finalized.
// Blocks B and C are created after block A has been finalized. They both observer key=1 value=1
//
// Block C observes:
// - key 1 value "swapped" from 1 to 2, because it was looking at finalized data
pub fn removed_fork_data_view<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState: TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>
        + Send
        + Sync
        + 'static,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default + 'static,
{
    let key = 1u64.to_be_bytes();
    let value_1 = vec![1u8; 32];
    let value_2 = vec![2u8; 32];

    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    // Just to put data into rocksdb
    let (block_a, block_b, block_c) = get_parent_and_2_children();

    // Operations in Block A
    let (storage, _) = storage_manager.create_state_for(&block_a).unwrap();
    let stf_changes = storage.materialize_from_key_value(key.to_vec(), Some(value_1.to_vec()));
    storage_manager
        .save_change_set(&block_a, stf_changes, SchemaBatch::default())
        .unwrap();
    storage_manager.finalize(&block_a).unwrap();
    assert!(storage_manager.is_empty());

    // Changes are in rocksdb now, creating readers
    let (stf_reader_b, _) = storage_manager.create_state_for(&block_b).unwrap();
    let (stf_reader_c, _) = storage_manager.create_state_for(&block_c).unwrap();
    //
    let value_at_b = stf_reader_b.get_value(&key);
    assert_eq!(Some(value_1.clone()), value_at_b);
    let value_at_c = stf_reader_c.get_value(&key);
    assert_eq!(Some(value_1.clone()), value_at_c);

    // Saving block B, data is correct
    let stf_changes = stf_reader_b.materialize_from_key_value(key.to_vec(), Some(value_2.to_vec()));
    storage_manager
        .save_change_set(&block_b, stf_changes, SchemaBatch::default())
        .unwrap();
    assert!(!storage_manager.is_empty());

    let value_at_c = stf_reader_c.get_value(&key);
    assert_eq!(Some(value_1.clone()), value_at_c);

    storage_manager.finalize(&block_b).unwrap();
    assert_eq!(storage_manager.snapshots_count(), 0);

    let value_at_c = stf_reader_c.get_value(&key);
    // Now it suddenly has `value_2`, instead of `value_1` that has been observed previously.
    assert_eq!(Some(value_2), value_at_c);
}

/// it is similar to [`linear_progression`], but it writes different data.
/// block 1 writes its own hash to all keys from 1 to max height.
/// block 2 writes its own hash to all keys from 1 to max_height - 1.
/// etc.
/// the last block only writes its own hash to key=1.
/// if snapshots are out of order, expected values will be "shadowed".
pub fn check_snapshots_ordering<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState: TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>
        + Send
        + Sync
        + 'static,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default + 'static,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    let to_height: u64 = 5;
    let mut expected_values = Vec::with_capacity(to_height as usize);

    for height in 1..=to_height {
        let da_header = MockBlockHeader::from_height(height);
        let hash_bytes = da_header.hash().0.to_vec();
        let write_to = to_height
            .checked_sub(height)
            .unwrap()
            .checked_add(1)
            .unwrap();
        expected_values.push((write_to.to_be_bytes().to_vec(), Some(hash_bytes)));

        let (stf_storage, _) = storage_manager.create_state_for(&da_header).unwrap();

        let stf_changes = stf_storage.materialize_from_key_values(&expected_values);

        storage_manager
            .save_change_set(&da_header, stf_changes, SchemaBatch::default())
            .unwrap();
    }

    let da_header = MockBlockHeader::from_height(to_height);
    let (stf_storage, _) = storage_manager.create_state_after(&da_header).unwrap();

    for (key, expected_value) in expected_values {
        let actual_value = stf_storage.get_value(&key);
        assert_eq!(actual_value, expected_value);
    }
}

pub async fn ledger_finalized_height_is_updated_on_start<Sm: TestableStorageManager>()
where
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfState: TestableStorage<ChangeSet = <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet>
        + Debug,
    <Sm as HierarchicalStorageManager<MockDaSpec>>::StfChangeSet: Default,
{
    let tmpdir = tempfile::tempdir().unwrap();
    let mut storage_manager = Sm::new(tmpdir.path());

    let blocks = 10;
    let finality = 5;

    for height in 0..=blocks {
        let da_header = MockBlockHeader::from_height(height);

        let (stf_state, ledger_reader) = storage_manager.create_state_for(&da_header).unwrap();
        drop(stf_state);

        let ledger_db = LedgerDb::with_reader(ledger_reader).unwrap();

        let slot_num = SlotNumber::new(height);

        let mut ledger_change_set = SchemaBatch::new();

        let slot_to_store = StoredSlot {
            hash: da_header.hash().into(),
            state_root: Default::default(),
            extra_data: vec![].into(),
            batches: BatchNumber(0)..BatchNumber(0),
            timestamp: da_header.time(),
        };

        ledger_db
            .put_slot(&slot_to_store, &slot_num, &mut ledger_change_set)
            .unwrap();

        if let Some(finalized_height) = height.checked_sub(finality) {
            let finalized_slot_materialized = ledger_db
                .materialize_latest_finalize_slot(slot_num, SlotNumber::new(finalized_height))
                .unwrap();
            ledger_change_set.merge(finalized_slot_materialized);
        }

        storage_manager
            .save_change_set(&da_header, Default::default(), ledger_change_set)
            .unwrap();

        if let Some(finalized_height) = height.checked_sub(finality) {
            let finalized_header = MockBlockHeader::from_height(finalized_height);
            storage_manager.finalize(&finalized_header).unwrap();
        }
    }

    drop(storage_manager);

    let mut storage_manager = Sm::new(tmpdir.path());
    let last_finalized_block_header = MockBlockHeader::from_height(blocks.saturating_sub(finality));

    let (_, ledger_reader) = storage_manager
        .create_state_after(&last_finalized_block_header)
        .unwrap();
    let ledger_db = LedgerDb::with_reader(ledger_reader).unwrap();

    let last_finalized_slot_from_ledger =
        ledger_db.get_latest_finalized_slot_number().await.unwrap();

    assert_eq!(
        last_finalized_block_header.height(),
        last_finalized_slot_from_ledger.get()
    );
}

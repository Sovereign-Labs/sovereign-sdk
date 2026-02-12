use std::collections::VecDeque;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use jmt::storage::HasPreimage;
use nomt::trie::KeyPath;
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sha2::Digest;
use sov_db::accessory_db::AccessoryDb;
use sov_db::config::RollupDbConfig;
use sov_db::historical_state::HistoricalStateReader;
use sov_db::namespaces::{KernelNamespace, UserNamespace};
use sov_db::schema::types::slot_key::SlotKey;
use sov_db::state_db::StateDb;
use sov_db::storage_manager::{
    NativeChangeSet, NativeStorageManager, NomtChangeSet, NomtStorageManager, StateFinishedSession,
};
use sov_db::test_utils::{
    build_data_to_materialize, get_block_hash, get_expected_chain_values,
    materialize_ledger_changes, verify_accessory_db, verify_ledger_storage, ForkDescription,
    ForkMap, TestNativeStorage, TestNomtStorage, H,
};
use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::storage::HierarchicalStorageManager;

// ---------------------------------------------------------------------------
// Per-backend helpers: Native
// ---------------------------------------------------------------------------

type NativeSm = NativeStorageManager<MockDaSpec, TestNativeStorage>;

fn materialize_native(storage: TestNativeStorage, da_header: &MockBlockHeader) -> NativeChangeSet {
    let height = da_header.height().to_be_bytes().to_vec();
    let hash_bytes = da_header.hash().0.to_vec();

    let key_hash = jmt::KeyHash::with::<H>(&height);
    let batch = vec![(key_hash, Some(hash_bytes.clone()))];
    let accessory_batch = vec![(height.clone(), Some(hash_bytes))];
    let preimages = vec![(key_hash, &height)];

    let materialized_preimages =
        StateDb::materialize_preimages(preimages.clone(), preimages).unwrap();

    let jmt_handler_user = storage.state.get_jmt_handler::<UserNamespace>();
    let jmt_handler_kernel = storage.state.get_jmt_handler::<KernelNamespace>();

    let data_user = build_data_to_materialize::<_, H>(
        &jmt_handler_user,
        SlotNumber::GENESIS.get(),
        batch.clone(),
    );
    let data_kernel =
        build_data_to_materialize::<_, H>(&jmt_handler_kernel, SlotNumber::GENESIS.get(), batch);

    let state_change_set = storage
        .state
        .materialize(&data_kernel, &data_user, Some(materialized_preimages))
        .unwrap();

    let accessory_change_set =
        AccessoryDb::materialize_values(accessory_batch, SlotNumber::GENESIS).unwrap();

    NativeChangeSet {
        state_change_set,
        accessory_change_set,
    }
}

fn verify_native(stf_storage: &TestNativeStorage, expected_values: &[(u64, MockHash)]) {
    // Check StateDb (user namespace)
    let jmt_handler = stf_storage.state.get_jmt_handler::<UserNamespace>();
    for (expected_height, expected_hash) in expected_values {
        let height_bytes = expected_height.to_be_bytes();
        let key_hash = jmt::KeyHash::with::<H>(&height_bytes);
        let pre_image = jmt_handler
            .preimage(key_hash)
            .unwrap()
            .expect("Missing preimage");
        assert_eq!(&pre_image, &height_bytes);
        let value = stf_storage
            .state
            .get_value_option_by_key::<UserNamespace>(SlotNumber::GENESIS, &pre_image)
            .expect("Failed to get value option from state db");
        assert_eq!(Some(expected_hash.0.to_vec()), value);
    }
    verify_accessory_db(&stf_storage.accessory_db, expected_values);
}

// ---------------------------------------------------------------------------
// Per-backend helpers: NOMT
// ---------------------------------------------------------------------------

type NomtSm = NomtStorageManager<MockDaSpec, H, TestNomtStorage>;

fn materialize_nomt(storage: TestNomtStorage, da_header: &MockBlockHeader) -> NomtChangeSet {
    let height = da_header.height().to_be_bytes().to_vec();
    let hash_bytes = da_header.hash().0.to_vec();

    let TestNomtStorage {
        state_session_builder,
        historical_state: _,
        accessory_db: _,
    } = storage;

    let user_session = state_session_builder
        .begin_user_session_without_witness()
        .unwrap();
    let kernel_session = state_session_builder
        .begin_kernel_session_without_witness()
        .unwrap();

    let key_path = KeyPath::from(sha2::Sha256::digest(&height));
    kernel_session.warm_up(key_path);
    user_session.warm_up(key_path);
    let state_writes = vec![(
        key_path,
        nomt::KeyReadWrite::Write(Some(hash_bytes.clone())),
    )];

    let user_finished = user_session.finish(state_writes.clone()).unwrap();
    let kernel_finished = kernel_session.finish(state_writes).unwrap();

    let accessory_change_set = AccessoryDb::materialize_values(
        vec![(height.clone(), Some(hash_bytes.clone()))],
        SlotNumber::GENESIS,
    )
    .unwrap();

    let root_hash = [
        user_finished.root().into_inner(),
        kernel_finished.root().into_inner(),
    ]
    .concat();
    let historical_change_set = HistoricalStateReader::materialize_values(
        std::iter::once((
            SlotKey::from_slice(&height),
            Some(hash_bytes.clone().into()),
        )),
        std::iter::once((SlotKey::from_slice(&height), Some(hash_bytes.into()))),
        root_hash,
        SlotNumber::new(da_header.height() - 1),
    )
    .unwrap();

    NomtChangeSet {
        state: StateFinishedSession::new(user_finished, kernel_finished),
        historical_state: historical_change_set,
        accessory: accessory_change_set,
        pinned_cache: None,
    }
}

fn verify_nomt(stf_storage: &TestNomtStorage, expected_values: &[(u64, MockHash)]) {
    for (expected_height, expected_hash) in expected_values {
        let key = expected_height.to_be_bytes().to_vec();
        let key_path = KeyPath::from(sha2::Sha256::digest(&key));
        let user_session = stf_storage
            .state_session_builder
            .begin_user_session_without_witness()
            .unwrap();
        let value = user_session.read(key_path).unwrap();
        assert_eq!(value, Some(expected_hash.0.to_vec()));
    }
    verify_accessory_db(&stf_storage.accessory_db, expected_values);
}

// ---------------------------------------------------------------------------
// Generic setup shared by both backends
// ---------------------------------------------------------------------------

const SUB_FORKS_COUNT: usize = 7;
const MAIN_FORK_LEN: u8 = 30;

fn build_fork_map() -> ForkMap {
    let sub_fork_start = (MAIN_FORK_LEN - 1) as u64;
    let fork_description = ForkDescription {
        start_height: 1,
        length: MAIN_FORK_LEN,
        child_forks: vec![
            ForkDescription {
                start_height: sub_fork_start,
                length: 1,
                child_forks: Vec::new(),
            };
            SUB_FORKS_COUNT
        ],
    };
    ForkMap::from(fork_description)
}

/// Fill the storage manager with fork data and prepare readers for each fork.
///
/// Returns `(storage_manager, readers)` where readers is a vec of
/// `(stf_storage, ledger_storage, expected_values)` for fork_id 1..=SUB_FORKS_COUNT.
fn setup_native(
    path: &std::path::Path,
    fork_map: &ForkMap,
) -> (
    NativeSm,
    Vec<(TestNativeStorage, DeltaReader, Vec<(u64, MockHash)>)>,
) {
    let mut sm: NativeSm = NativeStorageManager::new(path).unwrap();
    fill_storage_manager(&mut sm, fork_map, materialize_native);
    let readers = prepare_readers(&mut sm, fork_map);
    (sm, readers)
}

fn setup_nomt(
    path: &std::path::Path,
    fork_map: &ForkMap,
) -> (
    NomtSm,
    Vec<(TestNomtStorage, DeltaReader, Vec<(u64, MockHash)>)>,
) {
    let config = RollupDbConfig::default_in_path(path.to_path_buf());
    let mut sm: NomtSm = NomtStorageManager::new(config).unwrap();
    fill_storage_manager(&mut sm, fork_map, materialize_nomt);
    let readers = prepare_readers(&mut sm, fork_map);
    (sm, readers)
}

fn fill_storage_manager<Sm, F>(sm: &mut Sm, fork_map: &ForkMap, materialize: F)
where
    Sm: HierarchicalStorageManager<MockDaSpec, LedgerChangeSet = SchemaBatch>,
    Sm::StfState: Sized,
    F: Fn(Sm::StfState, &MockBlockHeader) -> Sm::StfChangeSet,
{
    let start = fork_map.get_start().expect("Empty chain-map");
    let mut next_blocks = VecDeque::new();
    next_blocks.push_back(start);
    while let Some(block_hash) = next_blocks.pop_front() {
        for child in fork_map.get_child_hashes(&block_hash) {
            next_blocks.push_back(child);
        }
        let da_header = fork_map.get_block_header(&block_hash).unwrap();
        let (stf_storage, _) = sm.create_state_for(da_header).unwrap();
        let stf_changes = materialize(stf_storage, da_header);
        let ledger_changes = materialize_ledger_changes(da_header);
        sm.save_change_set(da_header, stf_changes, ledger_changes)
            .unwrap();
    }
}

fn prepare_readers<Sm>(
    sm: &mut Sm,
    fork_map: &ForkMap,
) -> Vec<(Sm::StfState, DeltaReader, Vec<(u64, MockHash)>)>
where
    Sm: HierarchicalStorageManager<MockDaSpec, LedgerState = DeltaReader>,
{
    let total_forks = SUB_FORKS_COUNT + 1;
    let mut readers = Vec::with_capacity(total_forks);
    for fork_id in 1..=total_forks {
        let block_hash = get_block_hash(fork_id as u64, MAIN_FORK_LEN as u64);
        let block_header = fork_map.get_block_header(&block_hash).unwrap();
        let this_chain = fork_map.get_chain_up_to(block_header.clone());
        let expected_values = get_expected_chain_values(&this_chain);
        let (stf_storage, ledger_storage) = sm.create_state_after(block_header).unwrap();
        readers.push((stf_storage, ledger_storage, expected_values));
    }
    readers
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

fn bench_parallel_forks_native(c: &mut Criterion) {
    let fork_map = build_fork_map();
    let tmpdir = tempfile::tempdir().unwrap();
    let (mut sm, readers) = setup_native(tmpdir.path(), &fork_map);

    let mut group = c.benchmark_group("parallel_forks/native");

    // 1. single_read: single-threaded verification baseline
    group.bench_function("single_read", |b| {
        let (stf, ledger, expected) = &readers[0];
        b.iter(|| {
            verify_native(stf, expected);
            verify_ledger_storage(ledger, expected);
        });
    });

    // 2. concurrent_reads: 7 reader threads
    group.bench_function("concurrent_reads", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let start = std::time::Instant::now();
                std::thread::scope(|s| {
                    for (stf, ledger, expected) in &readers[1..] {
                        s.spawn(|| {
                            verify_native(stf, expected);
                            verify_ledger_storage(ledger, expected);
                        });
                    }
                });
                total += start.elapsed();
            }
            total
        });
    });

    // 3. concurrent_reads_during_finalization: readers + finalization
    group.sample_size(10);
    group.bench_function("concurrent_reads_during_finalization", |b| {
        b.iter_batched(
            || {
                // Per-iteration setup: rebuild state so finalization can run again.
                let fresh_tmpdir = tempfile::tempdir().unwrap();
                let (fresh_sm, fresh_readers) = setup_native(fresh_tmpdir.path(), &fork_map);
                (fresh_tmpdir, fresh_sm, fresh_readers)
            },
            |(_tmpdir, mut fresh_sm, fresh_readers)| {
                std::thread::scope(|s| {
                    for (stf, ledger, expected) in &fresh_readers[1..] {
                        s.spawn(|| {
                            verify_native(stf, expected);
                            verify_ledger_storage(ledger, expected);
                        });
                    }
                    // Main thread finalizes
                    for height in 1..MAIN_FORK_LEN {
                        let block_hash = get_block_hash(1, height as u64);
                        let block_header = fork_map.get_block_header(&block_hash).unwrap();
                        fresh_sm.finalize(block_header).unwrap();
                    }
                });
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();

    // Finalize the original storage manager to clean up
    for height in 1..MAIN_FORK_LEN {
        let block_hash = get_block_hash(1, height as u64);
        let block_header = fork_map.get_block_header(&block_hash).unwrap();
        sm.finalize(block_header).unwrap();
    }
}

fn bench_parallel_forks_nomt(c: &mut Criterion) {
    let fork_map = build_fork_map();
    let tmpdir = tempfile::tempdir().unwrap();
    let (mut sm, readers) = setup_nomt(tmpdir.path(), &fork_map);

    let mut group = c.benchmark_group("parallel_forks/nomt");

    // 1. single_read: single-threaded verification baseline
    group.bench_function("single_read", |b| {
        let (stf, ledger, expected) = &readers[0];
        b.iter(|| {
            verify_nomt(stf, expected);
            verify_ledger_storage(ledger, expected);
        });
    });

    // 2. concurrent_reads: 7 reader threads
    group.bench_function("concurrent_reads", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let start = std::time::Instant::now();
                std::thread::scope(|s| {
                    for (stf, ledger, expected) in &readers[1..] {
                        s.spawn(|| {
                            verify_nomt(stf, expected);
                            verify_ledger_storage(ledger, expected);
                        });
                    }
                });
                total += start.elapsed();
            }
            total
        });
    });

    // 3. concurrent_reads_during_finalization: readers + finalization
    group.sample_size(10);
    group.bench_function("concurrent_reads_during_finalization", |b| {
        b.iter_batched(
            || {
                // Per-iteration setup: rebuild state so finalization can run again.
                let fresh_tmpdir = tempfile::tempdir().unwrap();
                let (fresh_sm, fresh_readers) = setup_nomt(fresh_tmpdir.path(), &fork_map);
                (fresh_tmpdir, fresh_sm, fresh_readers)
            },
            |(_tmpdir, mut fresh_sm, fresh_readers)| {
                std::thread::scope(|s| {
                    for (stf, ledger, expected) in &fresh_readers[1..] {
                        s.spawn(|| {
                            verify_nomt(stf, expected);
                            verify_ledger_storage(ledger, expected);
                        });
                    }
                    // Main thread finalizes
                    for height in 1..MAIN_FORK_LEN {
                        let block_hash = get_block_hash(1, height as u64);
                        let block_header = fork_map.get_block_header(&block_hash).unwrap();
                        fresh_sm.finalize(block_header).unwrap();
                    }
                });
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();

    // Finalize the original storage manager to clean up
    for height in 1..MAIN_FORK_LEN {
        let block_hash = get_block_hash(1, height as u64);
        let block_header = fork_map.get_block_header(&block_hash).unwrap();
        sm.finalize(block_header).unwrap();
    }
}

criterion_group!(
    benches,
    bench_parallel_forks_native,
    bench_parallel_forks_nomt
);
criterion_main!(benches);

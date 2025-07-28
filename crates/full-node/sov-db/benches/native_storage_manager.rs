use std::collections::HashSet;

use criterion::measurement::WallTime;
use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion,
};
use jmt::storage::HasPreimage;
use jmt::KeyHash;
use rand::SeedableRng;
use rockbound::SchemaBatch;
use sov_db::namespaces::UserNamespace;
use sov_db::state_db::StateDb;
use sov_db::storage_manager::{NativeChangeSet, NativeStorageManager};
use sov_db::test_utils::{generate_more_random_bytes, TestNativeStorage};
use sov_mock_da::MockBlockHeader;
use sov_rollup_interface::storage::HierarchicalStorageManager;

type Da = sov_mock_da::MockDaSpec;
pub type N = UserNamespace;

struct TestData {
    random_key: Vec<u8>,
    non_existing_key: Vec<u8>,
    storage_manager: NativeStorageManager<Da, TestNativeStorage>,
}

// Only StateDb contains data. AccessoryDb and LedgerDb are empty.
// Just filling Preimages table with some key=hash(key)
fn setup_storage_manager(
    path: &std::path::Path,
    // Defines how many heights processed.
    rollup_height: u64,
    // Defines how many snapshots StorageManager hold. If larger than rollup_height
    // than no data is written on disk.
    fork_len: u64,
    // Defines how many new keys are inserted for each height.
    num_new_writes: usize,
    // Defines how many old keys are used in each height.
    num_old_writes: usize,
) -> TestData {
    assert_ne!(0, num_new_writes);
    let seed: [u8; 32] = [1; 32];
    let mut rng = rand::prelude::StdRng::from_seed(seed);

    let mut storage_manager = NativeStorageManager::new(path).unwrap();

    let mut old_keys: HashSet<Vec<u8>> =
        HashSet::with_capacity(rollup_height as usize * num_old_writes);

    for height in 1..=rollup_height {
        let block_header = MockBlockHeader::from_height(height);
        let _ = storage_manager.create_state_for(&block_header).unwrap();

        let mut new_keys = generate_more_random_bytes(&mut rng, num_new_writes, &old_keys);

        if !old_keys.is_empty() {
            new_keys.extend(old_keys.iter().take(num_old_writes).cloned());
        }

        let preimages = new_keys.iter().map(|key| {
            let key_hash = KeyHash::with::<sha2::Sha256>(&key);
            (key_hash, key)
        });
        let materialized_preimages = StateDb::materialize_preimages(Vec::new(), preimages).unwrap();

        old_keys.extend(new_keys.into_iter());

        let stf_changes = NativeChangeSet {
            state_change_set: materialized_preimages,
            ..Default::default()
        };

        storage_manager
            .save_change_set(&block_header, stf_changes, SchemaBatch::new())
            .unwrap();

        if height > fork_len {
            let old_block_header = MockBlockHeader::from_height(height - fork_len);
            storage_manager.finalize(&old_block_header).unwrap();
        }
    }

    let non_existing_key = generate_more_random_bytes(&mut rng, 1, &old_keys)
        .into_iter()
        .next()
        .unwrap();

    let random_key = old_keys.into_iter().next().unwrap();
    TestData {
        random_key,
        non_existing_key,
        storage_manager,
    }
}

fn bench_read(
    g: &mut BenchmarkGroup<WallTime>,
    rollup_height: u64,
    fork_len: u64,
    num_new_writes: usize,
    num_old_writes: usize,
) {
    let tmpdir = tempfile::tempdir().unwrap();
    let TestData {
        mut storage_manager,
        random_key,
        non_existing_key,
    } = setup_storage_manager(
        tmpdir.path(),
        rollup_height,
        fork_len,
        num_new_writes,
        num_old_writes,
    );

    let block = MockBlockHeader::from_height(rollup_height + 1);
    let (stf_storage, _) = storage_manager.create_state_for(&block).unwrap();

    let random_id = format!(
        "random/new_writes={num_new_writes}/old_writes={num_old_writes}/fork_len={fork_len}/height=",
    );
    let random_key_hash = KeyHash::with::<sha2::Sha256>(&random_key);
    let random_read_input = &(stf_storage.clone(), random_key_hash);

    g.bench_with_input(
        BenchmarkId::new(random_id, rollup_height),
        random_read_input,
        |b, i| {
            b.iter(|| {
                let (stf_storage, key_hash) = i;
                let jmt_handler = stf_storage.state.get_jmt_handler::<N>();
                let result = black_box(jmt_handler.preimage(*key_hash).unwrap());
                assert!(result.is_some());
                black_box(result);
            });
        },
    );

    let non_existing_id = format!(
        "non_existing/new_writes={num_new_writes}/old_writes={num_old_writes}/fork_len={fork_len}/height=",
    );
    let non_existing_key_hash = KeyHash::with::<sha2::Sha256>(&non_existing_key);
    let non_existing_input = &(stf_storage, non_existing_key_hash);

    g.bench_with_input(
        BenchmarkId::new(non_existing_id, rollup_height),
        non_existing_input,
        |b, i| {
            b.iter(|| {
                let (stf_storage, key_hash) = i;
                let jmt_handler = stf_storage.state.get_jmt_handler::<N>();
                let result = black_box(jmt_handler.preimage(*key_hash).unwrap());
                assert!(result.is_none());
                black_box(result);
            });
        },
    );
}

fn native_storage_manager_benchmark(c: &mut Criterion) {
    // Idea of the test is to fill key preimages table in state db with value and then read from there.
    // Key might be in on disk or in snapshot in memory.
    // We test how many snapshots are present by setting fork_len
    // Total number of unique key/value pairs is rollup_height * new_writes.
    // But snapshots can take number of old_writes from previous iterations. This will result w

    let mut group = c.benchmark_group("NativeStorageManager");
    // Just having 10% of old keys usage.
    let checks = [(1000, 100), (10_000, 1_000), (30_000, 3_000)];
    group.noise_threshold(0.3);
    for (new_writes, old_writes) in checks {
        // All data is on disk.
        bench_read(&mut group, 10, 0, new_writes, old_writes);
        // 20% of the data is in memory
        bench_read(&mut group, 10, 2, new_writes, old_writes);
        // 80% of the data is in memory
        bench_read(&mut group, 10, 8, new_writes, old_writes);
        // 100% of the data is in memory
        bench_read(&mut group, 10, 10, new_writes, old_writes);
    }
}

criterion_group!(benches, native_storage_manager_benchmark);
criterion_main!(benches);

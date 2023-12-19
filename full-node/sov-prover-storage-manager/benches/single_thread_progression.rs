extern crate criterion;

use std::sync::Arc;

use criterion::measurement::WallTime;
use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion,
};
use rand::prelude::SliceRandom;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use sov_mock_da::MockBlockHeader;
use sov_prover_storage_manager::ProverStorageManager;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::storage::{CacheKey, CacheValue, StorageKey};
use sov_state::{ArrayWitness, OrderedReadsAndWrites, Storage};

type Da = sov_mock_da::MockDaSpec;
type S = sov_state::DefaultStorageSpec;

fn generate_random_bytes<R: Rng>(
    rng: &mut R,
    count: usize,
    existing_key: &[Vec<u8>],
) -> Vec<Vec<u8>> {
    let mut samples: Vec<Vec<u8>> = Vec::with_capacity(count);

    while samples.len() < count {
        let inner_vec_size = rng.gen_range(32..=256);
        let storage_key: Vec<u8> = (0..inner_vec_size).map(|_| rng.gen::<u8>()).collect();
        if !existing_key.contains(&storage_key) {
            samples.push(storage_key);
        }
    }
    samples
}

struct TestData {
    random_key: Vec<u8>,
    non_existing_key: Vec<u8>,
    storage_manager: ProverStorageManager<Da, S>,
}

//TODO: Extend with auxiliary data
fn setup_storage(
    path: &std::path::Path,
    rollup_height: u64,
    fork_len: u64,
    num_new_writes: usize,
    num_old_writes: usize,
) -> TestData {
    let config = sov_state::config::Config {
        path: path.to_path_buf(),
    };

    let mut storage_manager = ProverStorageManager::<Da, S>::new(config).unwrap();

    let mut old_writes: Vec<Vec<u8>> = Vec::with_capacity(rollup_height as usize * num_old_writes);

    let seed: [u8; 32] = [1; 32];

    // Create an RNG with the specified seed
    let mut rng = StdRng::from_seed(seed);

    for h in 1..=rollup_height {
        let block_header = MockBlockHeader::from_height(h);
        let storage = storage_manager.create_storage_on(&block_header).unwrap();

        let new_keys = generate_random_bytes(&mut rng, num_new_writes, &old_writes);
        let new_values = generate_random_bytes(&mut rng, num_new_writes, &[]);

        let mut ordered_writes: Vec<(CacheKey, Option<CacheValue>)> =
            Vec::with_capacity(num_new_writes);

        if !old_writes.is_empty() {
            // Old writes
            old_writes.shuffle(&mut rng);

            let old_values = generate_random_bytes(&mut rng, num_old_writes, &[]);

            for (key, value) in old_writes
                .iter()
                .take(num_old_writes)
                .zip(old_values.into_iter())
            {
                ordered_writes.push((
                    CacheKey {
                        key: Arc::new(key.clone()),
                    },
                    Some(CacheValue {
                        value: Arc::new(value),
                    }),
                ));
            }
        }

        // New writes
        for (key, value) in new_keys.into_iter().zip(new_values.into_iter()) {
            old_writes.push(key.clone());
            ordered_writes.push((
                CacheKey { key: Arc::new(key) },
                Some(CacheValue {
                    value: Arc::new(value),
                }),
            ));
        }

        let state_operations = OrderedReadsAndWrites {
            ordered_reads: Default::default(),
            ordered_writes,
        };

        let witness = ArrayWitness::default();
        let (_, state_update) = storage
            .compute_state_update(state_operations, &witness)
            .unwrap();
        storage.commit(&state_update, &OrderedReadsAndWrites::default());

        storage_manager
            .save_change_set(&block_header, storage)
            .unwrap();

        if h > fork_len {
            let old_block_header = MockBlockHeader::from_height(h - fork_len);
            storage_manager.finalize(&old_block_header).unwrap();
        }
    }
    let non_existing_key = generate_random_bytes(&mut rng, 1, &old_writes)
        .pop()
        .unwrap();
    old_writes.shuffle(&mut rng);
    let random_key = old_writes.pop().unwrap();
    TestData {
        random_key,
        non_existing_key,
        storage_manager,
    }
}

fn bench_random_read(
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
        ..
    } = setup_storage(
        tmpdir.path(),
        rollup_height,
        fork_len,
        num_new_writes,
        num_old_writes,
    );
    let block = MockBlockHeader::from_height(rollup_height + 1);
    let storage = storage_manager.create_storage_on(&block).unwrap();
    let cache_key = CacheKey {
        key: Arc::new(random_key),
    };
    let storage_key = StorageKey::from(cache_key);
    let id = format!(
        "random/new_writes={}/old_writes={}/height=",
        num_new_writes, num_new_writes,
    );
    g.bench_with_input(
        BenchmarkId::new(id, rollup_height),
        &(storage, storage_key),
        |b, i| {
            b.iter(|| {
                let (storage, random_key) = i;
                let witness = ArrayWitness::default();
                let result = black_box(storage.get(random_key, None, &witness));
                assert!(result.is_some());
                black_box(result);
            })
        },
    );
}

fn bench_not_found_read(
    g: &mut BenchmarkGroup<WallTime>,
    rollup_height: u64,
    fork_len: u64,
    num_new_writes: usize,
    num_old_writes: usize,
) {
    let tmpdir = tempfile::tempdir().unwrap();
    let TestData {
        mut storage_manager,
        non_existing_key,
        ..
    } = setup_storage(
        tmpdir.path(),
        rollup_height,
        fork_len,
        num_new_writes,
        num_old_writes,
    );
    let block = MockBlockHeader::from_height(rollup_height + 1);
    let storage = storage_manager.create_storage_on(&block).unwrap();
    let cache_key = CacheKey {
        key: Arc::new(non_existing_key),
    };
    let storage_key = StorageKey::from(cache_key);
    let id = format!(
        "not_found/new_writes={}/old_writes={}/height",
        num_new_writes, num_old_writes,
    );
    g.bench_with_input(
        BenchmarkId::new(id, rollup_height),
        &(storage, storage_key),
        |b, i| {
            b.iter(|| {
                let (storage, random_key) = i;
                let witness = ArrayWitness::default();
                let result = black_box(storage.get(random_key, None, &witness));
                assert!(result.is_none());
                black_box(result);
            })
        },
    );
}

fn prover_storage_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("ProverStorage");
    let checks = [(100, 10), (1000, 100), (10_000, 1_000), (30_000, 3_000)];
    group.noise_threshold(0.3);
    for (new_writes, old_writes) in checks {
        bench_random_read(&mut group, 10, 8, new_writes, old_writes);
        bench_not_found_read(&mut group, 10, 8, new_writes, old_writes);
    }
}

criterion_group!(benches, prover_storage_benchmark);
criterion_main!(benches);

extern crate criterion;

use std::sync::{Arc, RwLock};

use criterion::measurement::WallTime;
use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion,
};
use jmt::storage::TreeWriter;
use jmt::{JellyfishMerkleTree, KeyHash};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use sov_db::state_db::StateDB;
use sov_schema_db::snapshot::{DbSnapshot, NoopQueryManager, ReadOnlyLock};

// TODO: Improve for collisions
fn generate_random_bytes(count: usize) -> Vec<Vec<u8>> {
    let seed: [u8; 32] = [1; 32];

    // Create an RNG with the specified seed
    let mut rng = StdRng::from_seed(seed);

    let mut samples: Vec<Vec<u8>> = Vec::with_capacity(count);

    for _ in 0..count {
        let inner_vec_size = rng.gen_range(32..=256);
        let storage_key: Vec<u8> = (0..inner_vec_size).map(|_| rng.gen::<u8>()).collect();
        samples.push(storage_key);
    }

    samples
}

struct TestData {
    largest_key: Vec<u8>,
    random_key: Vec<u8>,
    non_existing_key: Vec<u8>,
    db: StateDB<NoopQueryManager>,
}

fn prepare_data(size: usize) -> TestData {
    assert!(size > 0, "Do not generate empty TestData");
    let manager = ReadOnlyLock::new(Arc::new(RwLock::new(Default::default())));
    let db_snapshot = DbSnapshot::<NoopQueryManager>::new(0, manager);
    let db = StateDB::with_db_snapshot(db_snapshot).unwrap();
    db.inc_next_version();

    let mut raw_data = generate_random_bytes(size * 2 + 1);
    let non_existing_key = raw_data.pop().unwrap();
    let random_key = raw_data.first().unwrap().clone();
    let largest_key = raw_data
        .iter()
        .enumerate()
        .filter_map(|(i, elem)| if i % 2 == 0 { Some(elem) } else { None })
        .max()
        .unwrap()
        .clone();

    let mut key_preimages = Vec::with_capacity(size);
    let mut batch = Vec::with_capacity(size);

    for chunk in raw_data.chunks(2) {
        let key = &chunk[0];
        let value = chunk[1].clone();
        let key_hash = KeyHash::with::<sha2::Sha256>(&key);
        key_preimages.push((key_hash, key));
        batch.push((key_hash, Some(value)));
    }

    let jmt = JellyfishMerkleTree::<_, sha2::Sha256>::new(&db);

    let (_new_root, _update_proof, tree_update) = jmt
        .put_value_set_with_proof(batch, 1)
        .expect("JMT update must succeed");

    db.put_preimages(key_preimages).unwrap();

    db.write_node_batch(&tree_update.node_batch).unwrap();

    // Sanity check:
    let version = db.get_next_version() - 1;
    for chunk in raw_data.chunks(2) {
        let key = &chunk[0];
        let value = chunk[1].clone();
        let res = db.get_value_option_by_key(version, key).unwrap();
        assert_eq!(Some(value), res);
    }

    let random_value = db.get_value_option_by_key(version, &random_key).unwrap();
    assert!(random_value.is_some());

    TestData {
        largest_key,
        random_key,
        non_existing_key,
        db,
    }
}

fn bench_random_read(g: &mut BenchmarkGroup<WallTime>, size: usize) {
    let TestData { db, random_key, .. } = prepare_data(size);
    let version = db.get_next_version() - 1;
    g.bench_with_input(
        BenchmarkId::new("bench_random_read", size),
        &(db, random_key, version),
        |b, i| {
            b.iter(|| {
                let (db, key, version) = i;
                let result = black_box(db.get_value_option_by_key(*version, key).unwrap());
                assert!(result.is_some());
                black_box(result);
            })
        },
    );
}

fn bench_largest_read(g: &mut BenchmarkGroup<WallTime>, size: usize) {
    let TestData {
        db,
        largest_key: _largest_key,
        ..
    } = prepare_data(size);
    let version = db.get_next_version() - 1;
    g.bench_with_input(
        BenchmarkId::new("bench_largest_read", size),
        &(db, _largest_key, version),
        |b, i| {
            b.iter(|| {
                let (db, key, version) = i;
                let result = black_box(db.get_value_option_by_key(*version, key).unwrap());
                assert!(result.is_some());
                black_box(result);
            })
        },
    );
}

fn bench_not_found_read(g: &mut BenchmarkGroup<WallTime>, size: usize) {
    let TestData {
        db,
        non_existing_key,
        ..
    } = prepare_data(size);
    let version = db.get_next_version() - 1;
    g.bench_with_input(
        BenchmarkId::new("bench_not_found_read", size),
        &(db, non_existing_key, version),
        |b, i| {
            b.iter(|| {
                let (db, key, version) = i;
                let result = black_box(db.get_value_option_by_key(*version, key).unwrap());
                assert!(result.is_none());
                black_box(result);
            })
        },
    );
}

fn state_db_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("StateDB");
    group.noise_threshold(0.3);
    for size in [1000, 10_000, 30_000] {
        bench_random_read(&mut group, size);
        bench_not_found_read(&mut group, size);
        bench_largest_read(&mut group, size);
    }
    group.finish();
}

criterion_group!(benches, state_db_benchmark);
criterion_main!(benches);

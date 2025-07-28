extern crate criterion;

use std::sync::Arc;

use criterion::measurement::WallTime;
use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion,
};
use jmt::{KeyHash, Version};
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::{SchemaBatch, DB};
use sov_db::namespaces::KernelNamespace;
use sov_db::state_db::StateDb;
use sov_db::test_utils::{build_data_to_materialize, generate_random_bytes};

type N = sov_db::namespaces::UserNamespace;

struct TestData {
    largest_key: Vec<u8>,
    random_key: Vec<u8>,
    non_existing_key: Vec<u8>,
    db: StateDb,
}

// Data is only written to UserNamespace, we consider it enough for the benchmark.
fn put_data(state_db: &StateDb, raw_data: Vec<Vec<u8>>, version: Version) -> SchemaBatch {
    let mut key_preimages = Vec::with_capacity(raw_data.len());
    let mut batch = Vec::with_capacity(raw_data.len());

    for chunk in raw_data.chunks(2) {
        let key = &chunk[0];
        let value = chunk[1].clone();
        let key_hash = KeyHash::with::<sha2::Sha256>(&key);
        key_preimages.push((key_hash, key));
        batch.push((key_hash, Some(value)));
    }

    let preimages_batch = StateDb::materialize_preimages([], key_preimages).unwrap();

    // Writing empty data into kernel namespace to keep versions in sync
    let kernel_materialize = build_data_to_materialize::<_, sha2::Sha256>(
        &state_db.get_jmt_handler::<KernelNamespace>(),
        version,
        Vec::new(),
    );
    let user_materialize = build_data_to_materialize::<_, sha2::Sha256>(
        &state_db.get_jmt_handler::<KernelNamespace>(),
        version,
        batch,
    );
    state_db
        .materialize(
            &kernel_materialize,
            &user_materialize,
            Some(preimages_batch),
        )
        .unwrap()
}

fn prepare_data(size: usize, rocksdb: DB) -> TestData {
    assert!(size > 0, "Do not generate empty TestData");

    let rocksdb = Arc::new(rocksdb);
    let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
    let db = StateDb::with_delta_reader(reader).unwrap();

    let mut raw_data = generate_random_bytes(size * 2 + 1)
        .into_iter()
        .collect::<Vec<Vec<u8>>>();
    let non_existing_key = raw_data.pop().unwrap();
    let random_key = raw_data.first().unwrap().clone();
    let largest_key = raw_data
        .iter()
        .enumerate()
        .filter_map(|(i, elem)| if i % 2 == 0 { Some(elem) } else { None })
        .max()
        .unwrap()
        .clone();

    let version = 0;
    let data = put_data(&db, raw_data.clone(), version);

    rocksdb.write_schemas(&data).unwrap();

    // re-initialize `StateDb` so the latest version is updated.
    let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
    let db = StateDb::with_delta_reader(reader).unwrap();
    let version = db
        .get_next_version()
        .checked_sub(1)
        .expect("Should have data write data");
    for chunk in raw_data.chunks(2) {
        let key = &chunk[0];
        let value = chunk[1].clone();
        let res = db.get_value_option_by_key::<N>(version, key).unwrap();
        assert_eq!(Some(value), res);
    }

    let random_value = db
        .get_value_option_by_key::<N>(version, &random_key)
        .unwrap();
    assert!(random_value.is_some());

    TestData {
        largest_key,
        random_key,
        non_existing_key,
        db,
    }
}

fn bench_random_read(g: &mut BenchmarkGroup<WallTime>, size: usize) {
    let tempdir = tempfile::tempdir().unwrap();
    let state_rocksdb = StateDb::get_rockbound_options()
        .default_setup_db_in_path(tempdir.path())
        .unwrap();
    let TestData { db, random_key, .. } = prepare_data(size, state_rocksdb);
    let version = db.last_version().unwrap();
    g.bench_with_input(
        BenchmarkId::new("bench_random_read", size),
        &(db, random_key, version),
        |b, i| {
            b.iter(|| {
                let (db, key, version) = i;
                let result = black_box(db.get_value_option_by_key::<N>(*version, key).unwrap());
                assert!(result.is_some());
                black_box(result);
            });
        },
    );
}

fn bench_largest_read(g: &mut BenchmarkGroup<WallTime>, size: usize) {
    let tempdir = tempfile::tempdir().unwrap();
    let state_rocksdb = StateDb::get_rockbound_options()
        .default_setup_db_in_path(tempdir.path())
        .unwrap();
    let TestData {
        db,
        largest_key: _largest_key,
        ..
    } = prepare_data(size, state_rocksdb);
    let version = db.last_version().unwrap();
    g.bench_with_input(
        BenchmarkId::new("bench_largest_read", size),
        &(db, _largest_key, version),
        |b, i| {
            b.iter(|| {
                let (db, key, version) = i;
                let result = black_box(db.get_value_option_by_key::<N>(*version, key).unwrap());
                assert!(result.is_some());
                black_box(result);
            });
        },
    );
}

fn bench_not_found_read(g: &mut BenchmarkGroup<WallTime>, size: usize) {
    let tempdir = tempfile::tempdir().unwrap();
    let state_rocksdb = StateDb::get_rockbound_options()
        .default_setup_db_in_path(tempdir.path())
        .unwrap();
    let TestData {
        db,
        non_existing_key,
        ..
    } = prepare_data(size, state_rocksdb);
    let version = db.last_version().unwrap();
    g.bench_with_input(
        BenchmarkId::new("bench_not_found_read", size),
        &(db, non_existing_key, version),
        |b, i| {
            b.iter(|| {
                let (db, key, version) = i;
                let result = black_box(db.get_value_option_by_key::<N>(*version, key).unwrap());
                assert!(result.is_none());
                black_box(result);
            });
        },
    );
}

fn state_db_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("StateDb");
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

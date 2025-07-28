#![allow(clippy::float_arithmetic)]
use std::fs;
use std::path::Path;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};
use sov_db::accessory_db::AccessoryDb;
use sov_db::pruner::Pruner;
use sov_db::test_utils::{fill_accessory_db, VersionDistribution};
use sov_rollup_interface::common::SlotNumber;

// Function to calculate directory size recursively
fn dir_size(path: &Path) -> std::io::Result<u64> {
    let mut size = 0;

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                size += dir_size(&path)?;
            } else {
                size += entry.metadata()?.len();
            }
        }
    }

    Ok(size)
}

fn bench_pruner(c: &mut Criterion) {
    let mut group = c.benchmark_group("pruner");
    group.sample_size(11);
    group.bench_function("pruner_basic", |b| {
        b.iter_with_setup(
            // Setup: create temp DB and fill with data
            || {
                let tempdir = tempfile::tempdir().unwrap();
                let rocksdb = Arc::new(
                    AccessoryDb::get_rockbound_options()
                        .default_setup_db_in_path(tempdir.path())
                        .unwrap(),
                );

                fill_accessory_db(
                    &rocksdb,
                    10_000,
                    VersionDistribution::Distributed {
                        profiles: vec![(0.2, 3_000), (0.3, 5_000), (0.5, 8_000)],
                    },
                    Some(SlotNumber::new(0)),
                    "This/Is/Longer/Path/For/Benchmark/To/See/Impact_",
                )
                .unwrap();

                let size_bytes = dir_size(tempdir.path()).unwrap();
                println!(
                    "Size of tempdir after fill_accessory_db: {:.2} MB",
                    size_bytes as f64 / (1024.0 * 1024.0)
                );

                (tempdir, rocksdb)
            },
            |(_tempdir, rocksdb)| {
                let pruner = Pruner::new(rocksdb.clone());
                let pruning_batch = pruner
                    .collect_pruning_batch_for_module_accessory_state(100)
                    .unwrap();
                rocksdb.write_schemas(&pruning_batch).unwrap();
            },
        );
    });
    group.finish();
}

criterion_group!(benches, bench_pruner);
criterion_main!(benches);

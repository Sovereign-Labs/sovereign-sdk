//! Native calibration of the storage-access gas constants.
//!
//! The SP1 storage microbench prices a state access by the *prover* cost of
//! verifying a NOMT merkle proof of worst-case depth. That's the right basis
//! for a proven rollup but meaningless natively: there is no proof to verify,
//! and the binding resource is node execution time. This bench measures the
//! native wall-clock of a *real* access through the production NOMT + rocksdb
//! stack instead.
//!
//! The rollup is native-only and never proves, so storage is committed with
//! witness generation **off** ([`SimpleStorageManager::set_generate_witness`]) —
//! the NOMT `WitnessMode::On` default records merkle-proof hints purely to
//! support ZK proving, which a non-proving node never pays.
//!
//! Two sweeps:
//!   - `storage_read` — cold read (fresh working set → NOMT/rocksdb lookup), a
//!     value-size sweep fitted as `ns = bias + per_byte * size`. The base maps
//!     to `GAS_TO_CHARGE_PER_READ`, the slope to `GAS_TO_CHARGE_PER_BYTE_READ`.
//!   - `storage_write` — marginal write cost: a *writes-per-commit* sweep fitted
//!     as `ns = fixed + per_write * count`, taking the SLOPE as
//!     `BIAS_STORAGE_UPDATE`. The intercept is the once-per-block commit (NOMT
//!     root finalization + rocksdb fsync), a fixed cost independent of write
//!     count that block-production economics — not per-tx gas — bound, so it
//!     drops out of the slope.
//!
//! `GAS_TO_CHARGE_PER_STORAGE_ACCESS` is charged on both reads and writes; we
//! fold the whole measured native cost into the read/write-specific constants
//! and leave it at the floor (1). The hash sub-charges in `charge_read` /
//! `charge_write` are owned by the sha256 bench — the tiny key/value hash time
//! baked into these numbers only double-counts them within the noise.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use sov_gas_tools::report::{report_marginal_sweep, report_size_sweep};
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::{StateCheckpoint, StateMap};
use sov_state::{BorshCodec, Prefix, Storage};
use sov_test_utils::storage::{ForklessStorageManager, SimpleStorageManager};
use sov_test_utils::{write_kernel_marker, TestSpec, TestStorageSpec};
use unwrap_infallible::UnwrapInfallible;

type S = TestSpec;
type Manager = SimpleStorageManager<TestStorageSpec>;

// Read value-size sweep. State values are usually small, so cap well below the
// crypto benches' 64 KiB.
const SIZES: &[u64] = &[0, 8, 32, 64, 128, 256, 512, 1024, 4096];

// Writes-per-commit sweep. The slope of commit-time vs count is the marginal
// per-write cost; the range sets a tree of ~count keys (depth ~log2(count)).
const WRITE_COUNTS: &[u64] = &[256, 512, 1024, 2048, 4096];
// Fixed small value for the write sweep — value bytes don't move commit cost
// (NOMT hashes them to 32 bytes), so they're held constant here.
const WRITE_VALUE_SIZE: u64 = 32;

fn value_of(size: u64) -> Vec<u8> {
    (0..size).map(|i| (i as u8).wrapping_mul(0xAB)).collect()
}

/// A fresh NOMT-backed manager with witness generation off (native-only).
fn new_manager() -> Manager {
    let mut manager = Manager::new();
    manager.set_generate_witness(false);
    manager
}

fn map() -> StateMap<u32, Vec<u8>> {
    StateMap::with_codec(Prefix::new(0, 0), BorshCodec)
}

/// Set `keys[start..start+count]` to `value` and commit them as one block,
/// advancing the manager's root and the kernel height.
fn write_block(
    manager: &mut Manager,
    kernel: &mut MockKernel<S>,
    start: u32,
    count: u32,
    value: &Vec<u8>,
) {
    let prev_root = manager.current_root();
    let storage = manager.create_prover_storage();
    let mut state = StateCheckpoint::<S>::new(storage.clone(), kernel);

    let mut m = map();
    for k in start..start + count {
        m.set(&k, value, &mut state).unwrap_infallible();
    }
    write_kernel_marker(&mut state).unwrap_infallible();
    let (cache_log, _, witness) = state.freeze();
    let (new_root, state_update) = storage
        .compute_state_update(cache_log, &witness, prev_root)
        .expect("compute_state_update must succeed natively");
    manager.commit_state_update(storage, state_update, new_root);
    kernel.increase_heights();
}

/// Cold read: a fresh working set per timed read, so the per-tx cache is empty
/// and the access really hits the NOMT session / rocksdb.
fn bench_storage_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_read");
    for &size in SIZES {
        // Pre-commit `key 0 = value` once; every iteration reads it back cold.
        let mut manager = new_manager();
        let mut kernel = MockKernel::<S>::default();
        write_block(&mut manager, &mut kernel, 0, 1, &value_of(size));

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter_batched(
                || StateCheckpoint::<S>::new(manager.create_prover_storage(), &kernel),
                |mut state| {
                    black_box(
                        map()
                            .get(black_box(&0u32), &mut state)
                            .unwrap_infallible(),
                    );
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();

    if let Err(e) = report_size_sweep(
        "storage_read",
        SIZES,
        "GAS_TO_CHARGE_PER_READ",
        "GAS_TO_CHARGE_PER_BYTE_READ",
    ) {
        eprintln!("warn: could not compute fit / constants suggestion: {e}");
    }
}

/// Marginal write cost. Sweeps the number of writes committed in one block and
/// fits `commit_time = fixed + per_write * count`; the slope is the marginal
/// cost a write imposes (the trie path it dirties), while the intercept — the
/// once-per-block fsync / root finalization — drops out and is not charged.
/// Each measurement commits into a fresh DB so the tree starts empty.
fn bench_storage_write(c: &mut Criterion) {
    let value = value_of(WRITE_VALUE_SIZE);
    let mut group = c.benchmark_group("storage_write");
    // Each iteration builds a fresh DB and does a full commit (~100 ms), so the
    // default 100 samples would take far too long.
    group.sample_size(10);
    for &count in WRITE_COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                new_manager,
                |mut manager| {
                    let mut kernel = MockKernel::<S>::default();
                    write_block(&mut manager, &mut kernel, 0, count as u32, &value);
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();

    if let Err(e) = report_marginal_sweep("storage_write", WRITE_COUNTS, "BIAS_STORAGE_UPDATE") {
        eprintln!("warn: could not compute fit / constants suggestion: {e}");
    }
    eprintln!(
        "note: GAS_TO_CHARGE_PER_BYTE_STORAGE_UPDATE floors to 1 (NOMT hashes values to \
         32 bytes, so value bytes don't move commit cost — that cost is already priced \
         by the hash constants); GAS_TO_CHARGE_PER_STORAGE_ACCESS stays [1, 0]."
    );
}

criterion_group!(benches, bench_storage_read, bench_storage_write);
criterion_main!(benches);

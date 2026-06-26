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
//!   - `storage_write_perbyte` — a value-size sweep at a fixed, byte-bounded
//!     write count (so commits stay a few MiB and avoid large-batch rocksdb flush
//!     pathology). The commit-vs-size slope is `count * per_byte`;
//!     `GAS_TO_CHARGE_PER_BYTE_STORAGE_UPDATE` is that, netted against the value-
//!     hash rate.
//!
//! `BIAS_STORAGE_UPDATE` (the size-independent per-write cost) is measured in the
//! separate `storage_depth` bench: it grows with trie depth, so it must be taken
//! at production scale (~depth 24), which needs a far larger tree than these
//! criterion benches build.
//!
//! `GAS_TO_CHARGE_PER_STORAGE_ACCESS` is charged on both reads and writes; we
//! fold the whole measured native cost into the read/write-specific constants
//! and leave it at the floor (1).
//!
//! Hashing and borsh decode are billed separately (the sha256 and borsh
//! constants). The *write* per-byte contains hashing — the value is hashed to
//! build its trie leaf — so it's netted against the hash rate (residual
//! storage-I/O per-byte ~0). The *read* is measured through the container `get`
//! for simplicity, so its per-byte slightly overlaps `BORSH_PER_BYTE_READ`; the
//! overlap is within measurement noise. Reads don't hash the value in release
//! (the strict cross-check is `cfg!(debug_assertions)`-gated).

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use sov_gas_tools::report::{report_perbyte_sweep, report_size_sweep};
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

// per_byte_hash_update (sha256 native ≈ 0.40 ns/byte). The write hashes the value
// to build its trie leaf; that per-byte hashing is charged separately via the hash
// constants, so it's netted out of the raw write per-byte.
const HASH_NS_PER_BYTE: f64 = 0.40;

// Per-byte cost: sweep value size at a FIXED write count, keeping bytes-per-commit
// bounded (count * max_size ≈ 4 MiB) so the commit avoids the large-batch rocksdb
// flush pathology that inflated the earlier 128 MiB sweep. The slope of commit-time
// vs size is `count * per_byte`.
const WRITE_PERBYTE_COUNT: u64 = 256;
const WRITE_PERBYTE_SIZES: &[u64] = &[0, 4096, 8192, 16384]; // 256 * 16384 ≈ 4 MiB/commit max

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

/// Cold read: a fresh checkpoint per timed read, so the per-tx cache is empty
/// and the access really hits the NOMT session / rocksdb.
fn bench_storage_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_read");
    for &size in SIZES {
        let mut manager = new_manager();
        let mut kernel = MockKernel::<S>::default();
        write_block(&mut manager, &mut kernel, 0, 1, &value_of(size));

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter_batched(
                || StateCheckpoint::<S>::new(manager.create_prover_storage(), &kernel),
                |mut state| {
                    black_box(map().get(black_box(&0u32), &mut state).unwrap_infallible());
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

/// Helper: bench one fixed-size write block of `count` writes, fresh DB per
/// measurement so the tree starts empty.
fn bench_write_block(b: &mut criterion::Bencher, count: u32, value: &Vec<u8>) {
    b.iter_batched(
        new_manager,
        |mut manager| {
            let mut kernel = MockKernel::<S>::default();
            write_block(&mut manager, &mut kernel, 0, count, value);
        },
        BatchSize::PerIteration,
    );
}

/// Per-byte write cost. Sweep value size at a fixed, byte-bounded write count;
/// the slope of commit-time vs size is `count * per_byte`, fsync as the
/// discarded intercept. Netted against the value-hash rate (charged separately).
fn bench_storage_write_perbyte(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_write_perbyte");
    group.sample_size(10);
    for &size in WRITE_PERBYTE_SIZES {
        let value = value_of(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            bench_write_block(b, WRITE_PERBYTE_COUNT as u32, &value);
        });
    }
    group.finish();

    if let Err(e) = report_perbyte_sweep(
        "storage_write_perbyte",
        WRITE_PERBYTE_SIZES,
        WRITE_PERBYTE_COUNT,
        HASH_NS_PER_BYTE,
        "GAS_TO_CHARGE_PER_BYTE_STORAGE_UPDATE",
    ) {
        eprintln!("warn: could not compute fit / constants suggestion: {e}");
    }
    eprintln!(
        "note: GAS_TO_CHARGE_PER_STORAGE_ACCESS stays [1, 0] (shared per-access bias \
         folded into the read/write constants)."
    );
}

criterion_group!(benches, bench_storage_read, bench_storage_write_perbyte);
criterion_main!(benches);

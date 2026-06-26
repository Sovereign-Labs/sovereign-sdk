//! Depth sweep — how the marginal per-write cost grows with NOMT trie depth.
//!
//! `storage_write_fixed` measures the per-write cost on a shallow test tree
//! (~2K keys, depth ~11). Production trees hold millions of keys (depth ~24), and
//! a write re-hashes and re-reads its full root-to-leaf path, so the per-write
//! cost grows with depth. This builds one tree incrementally toward 2^24 keys,
//! pausing at checkpoints to measure the fsync-free per-write (two-batch
//! differencing), then fits per-write vs depth and extrapolates
//! `BIAS_STORAGE_UPDATE` to depth 24.
//!
//! Empty values isolate the depth-dependent path cost from value movement.
//! Slow — builds ~16M keys, several minutes. Run with `TMPDIR` on a real disk.
//!
//! Host-only post-processing; the workspace `float_arithmetic` deny (which guards
//! native/zkVM divergence) doesn't apply.
#![allow(clippy::float_arithmetic)]

use std::time::Instant;

use sov_gas_tools::fit::fit_linear;
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::{StateCheckpoint, StateMap};
use sov_state::{BorshCodec, Prefix, Storage};
use sov_test_utils::storage::{ForklessStorageManager, SimpleStorageManager};
use sov_test_utils::{write_kernel_marker, TestSpec, TestStorageSpec};
use unwrap_infallible::UnwrapInfallible;

type S = TestSpec;
type Manager = SimpleStorageManager<TestStorageSpec>;

const BUILD_CHUNK: u32 = 65_536;
// Key counts at which to measure; depth ≈ log2(keys): ~14, ~17, ~20, ~22, ~24.
const CHECKPOINTS: &[u64] = &[1 << 14, 1 << 17, 1 << 20, 1 << 22, 1 << 24];
const DIFF_LO: u32 = 256;
const DIFF_HI: u32 = 2048;
const REPS: u32 = 3;

fn new_manager() -> Manager {
    let mut m = Manager::new();
    m.set_generate_witness(false);
    m
}

/// Commit `count` empty-valued keys starting at `start` into the tree.
fn write_block(manager: &mut Manager, kernel: &mut MockKernel<S>, start: u32, count: u32) {
    let prev_root = manager.current_root();
    let storage = manager.create_prover_storage();
    let mut state = StateCheckpoint::<S>::new(storage.clone(), kernel);
    let mut map: StateMap<u32, Vec<u8>> = StateMap::with_codec(Prefix::new(0, 0), BorshCodec);
    let empty: Vec<u8> = Vec::new();
    for k in start..start + count {
        map.set(&k, &empty, &mut state).unwrap_infallible();
    }
    write_kernel_marker(&mut state).unwrap_infallible();
    let (cache_log, _, witness) = state.freeze();
    let (new_root, state_update) = storage
        .compute_state_update(cache_log, &witness, prev_root)
        .expect("compute_state_update must succeed natively");
    manager.commit_state_update(storage, state_update, new_root);
    kernel.increase_heights();
}

fn time_commit(manager: &mut Manager, kernel: &mut MockKernel<S>, start: u32, count: u32) -> f64 {
    let t = Instant::now();
    write_block(manager, kernel, start, count);
    t.elapsed().as_nanos() as f64
}

/// fsync-free per-write at the current depth: two batch sizes differenced (the
/// once-per-commit fsync cancels), averaged over `REPS`. Advances `next`.
fn measure_per_write(manager: &mut Manager, kernel: &mut MockKernel<S>, next: &mut u32) -> f64 {
    let mut acc = 0.0;
    for _ in 0..REPS {
        let lo = time_commit(manager, kernel, *next, DIFF_LO);
        *next += DIFF_LO;
        let hi = time_commit(manager, kernel, *next, DIFF_HI);
        *next += DIFF_HI;
        acc += (hi - lo) / f64::from(DIFF_HI - DIFF_LO);
    }
    acc / f64::from(REPS)
}

fn main() {
    let mut manager = new_manager();
    let mut kernel = MockKernel::<S>::default();
    let mut next: u32 = 0;

    let mut depths = Vec::new();
    let mut per_write = Vec::new();

    println!("building tree incrementally; measuring per-write at each checkpoint\n");
    for &target in CHECKPOINTS {
        while u64::from(next) < target {
            let remaining = (target - u64::from(next)).min(u64::from(BUILD_CHUNK)) as u32;
            write_block(&mut manager, &mut kernel, next, remaining);
            next += remaining;
        }
        let depth = f64::from(next).log2();
        let pw = measure_per_write(&mut manager, &mut kernel, &mut next);
        println!(
            "  keys={next:>9}  depth~{depth:>4.1}   per_write = {:>7.2} us   (BIAS gas = {})",
            pw / 1000.0,
            (pw / 0.01).ceil() as u64
        );
        depths.push(depth);
        per_write.push(pw);
    }

    // Per-write rises with depth then plateaus, so a linear fit is a poor model
    // (printed only as a cross-check). The deepest checkpoint reaches production
    // depth (~24) directly — use that measured value as the constant.
    let deepest_depth = *depths.last().expect("at least one checkpoint");
    let deepest_pw = *per_write.last().expect("at least one checkpoint");
    let fit = fit_linear(&depths, &per_write).expect("depth fit");
    println!(
        "\n(linear cross-check: per_write(ns) = {:.0} + {:.0} * depth, R²={:.4} — low R² \
         reflects the plateau; prefer the direct measurement below)",
        fit.bias, fit.per_byte, fit.r_squared
    );
    println!(
        "=> BIAS_STORAGE_UPDATE at depth ~{deepest_depth:.0} (production scale) = [{}, 0]  ({:.1} us/write)",
        (deepest_pw / 0.01).ceil() as u64,
        deepest_pw / 1000.0,
    );
}

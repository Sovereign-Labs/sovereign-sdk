//! Benchmark suite comparing four transaction uniqueness approaches:
//!
//! 1. **Nonce** (Ethereum-style): Strict sequential counter
//! 2. **Generation** (current production): BTreeMap<u64, HashSet<TxHash>> with Borsh ser/de
//! 3. **Window v1** (PR #2633): Dynamic Vec<u8> bitfield
//! 4. **Window v2** (improved): Fixed-size [u64; 16] bitfield — zero allocation
//!
//! Run with: `cargo bench -p sov-uniqueness`

use std::collections::{BTreeMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

// ============================================================================
// Approach 1: Ethereum-style Nonce (sequential counter)
// ============================================================================

#[derive(Clone, Default, BorshSerialize, BorshDeserialize)]
struct NonceState {
    next_nonce: u64,
}

impl NonceState {
    fn check(&self, nonce: u64) -> bool {
        nonce == self.next_nonce
    }

    fn mark(&mut self, _nonce: u64) {
        self.next_nonce += 1;
    }
}

// ============================================================================
// Approach 2: Generation Numbers (current production — mirrors generations.rs)
// ============================================================================

const PAST_TRANSACTION_GENERATIONS: u64 = 5_000;
const MAX_STORED_TX_HASHES: u64 = 1_700;

#[derive(Clone, Default, BorshSerialize, BorshDeserialize)]
struct GenerationState {
    buckets: BTreeMap<u64, HashSet<[u8; 32]>>,
}

impl GenerationState {
    fn check(&self, generation: u64, tx_hash: [u8; 32]) -> bool {
        let latest = self
            .buckets
            .keys()
            .next_back()
            .copied()
            .unwrap_or(generation);

        // Check generation is within valid range
        let cutoff = PAST_TRANSACTION_GENERATIONS.saturating_sub(1);
        if latest.saturating_sub(cutoff) > generation {
            return false;
        }

        // Check hash not already seen
        if let Some(bucket) = self.buckets.get(&generation) {
            if bucket.contains(&tx_hash) {
                return false;
            }
        }

        // Check total count won't exceed limit (simplified — mirrors the real logic)
        let mut test_buckets = self.buckets.clone();
        if generation > latest {
            let lower_bound = generation.saturating_sub(PAST_TRANSACTION_GENERATIONS);
            test_buckets = test_buckets.split_off(&lower_bound);
        }
        let total: u64 = test_buckets.values().map(|b| b.len() as u64).sum();
        total + 1 <= MAX_STORED_TX_HASHES
    }

    fn mark(&mut self, generation: u64, tx_hash: [u8; 32]) {
        let latest = self
            .buckets
            .keys()
            .next_back()
            .copied()
            .unwrap_or(generation);

        if generation > latest {
            let lower_bound = generation.saturating_sub(PAST_TRANSACTION_GENERATIONS);
            self.buckets = self.buckets.split_off(&lower_bound);
        }

        self.buckets.entry(generation).or_default().insert(tx_hash);
    }
}

// ============================================================================
// Approach 3: Window v1 (PR #2633 — Vec<u8> bitfield)
// ============================================================================

const PAST_TRANSACTION_WINDOW_V1: u64 = 1_000;

#[derive(Clone, Default, BorshSerialize, BorshDeserialize)]
struct WindowV1State {
    start: u64,
    bits: Vec<u8>,
}

impl WindowV1State {
    fn check(&self, nonce: u64) -> bool {
        if nonce < self.start {
            return false;
        }
        let delta = (nonce - self.start) as usize;
        let v = self.bits.get(delta / 8).copied().unwrap_or(0);
        v & (1 << (delta % 8)) == 0
    }

    fn mark(&mut self, nonce: u64) {
        assert!(nonce >= self.start);

        // Drop outdated bits at the front
        let drop = (nonce - self.start).saturating_sub(PAST_TRANSACTION_WINDOW_V1) / 8;
        self.bits = self.bits.split_off((drop as usize).min(self.bits.len()));
        self.start += drop * 8;

        // Expand to fit
        let delta = (nonce - self.start) as usize;
        self.bits.resize((delta / 8 + 1).max(self.bits.len()), 0);

        // Mark bit
        self.bits[delta / 8] |= 1 << (delta % 8);
    }
}

// ============================================================================
// Approach 4: Window v2 (improved — fixed-size [u64; 16] bitfield)
// ============================================================================

/// Fixed window size in bits. 1024 bits = 128 bytes = 16 u64s.
/// Supports tracking 1024 concurrent in-flight nonces within the window.
const WINDOW_BITS: usize = 1024;
const WINDOW_U64S: usize = WINDOW_BITS / 64;

/// Threshold at which we auto-advance the window. When a nonce lands in
/// the upper quarter of the window, we slide forward to re-center it.
const ADVANCE_THRESHOLD: usize = WINDOW_BITS * 3 / 4;
const ADVANCE_TARGET: usize = WINDOW_BITS / 2;

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct WindowV2State {
    start: u64,
    bits: [u64; WINDOW_U64S],
}

impl Default for WindowV2State {
    fn default() -> Self {
        Self {
            start: 0,
            bits: [0u64; WINDOW_U64S],
        }
    }
}

impl WindowV2State {
    /// Check if a nonce is valid (not duplicate, not too old, not too far ahead).
    /// This is the read-only validation path — no state mutation.
    #[inline]
    fn check(&self, nonce: u64) -> Result<(), &'static str> {
        if nonce < self.start {
            return Err("nonce too old");
        }
        // Compare as u64 BEFORE casting to usize to prevent truncation on 32-bit targets.
        let delta_u64 = nonce - self.start;
        if delta_u64 >= WINDOW_BITS as u64 {
            return Err("nonce too far ahead");
        }
        let delta = delta_u64 as usize; // safe: verified < WINDOW_BITS
        let word_idx = delta / 64;
        let bit_idx = delta % 64;
        if self.bits[word_idx] & (1u64 << bit_idx) != 0 {
            return Err("duplicate nonce");
        }
        Ok(())
    }

    /// Mark a nonce as used. MUST be called after check() passes.
    /// check() and mark() enforce the SAME acceptance policy: nonces outside
    /// [start, start + WINDOW_BITS) are always rejected. This prevents an
    /// attacker from using mark() to force window advancement and expire
    /// replay protection bits for recent nonces.
    #[inline]
    fn mark(&mut self, nonce: u64) -> Result<(), &'static str> {
        if nonce < self.start {
            return Err("nonce too old");
        }
        // Compare as u64 BEFORE casting to usize to prevent truncation on 32-bit targets.
        let delta_u64 = nonce - self.start;
        if delta_u64 >= WINDOW_BITS as u64 {
            return Err("nonce too far ahead");
        }
        let mut delta = delta_u64 as usize; // safe: verified < WINDOW_BITS

        // Auto-advance when nonce is in the upper quarter of the window.
        // This slides the window forward to maintain headroom for future nonces
        // while preserving ~512 bits of lookback for recent nonces.
        if delta >= ADVANCE_THRESHOLD {
            let advance = delta - ADVANCE_TARGET;
            self.shift_right(advance);
            // Use checked_add to prevent overflow near u64::MAX
            self.start = self
                .start
                .checked_add(advance as u64)
                .ok_or("nonce space exhausted")?;
            delta = (nonce - self.start) as usize;
        }

        let word_idx = delta / 64;
        let bit_idx = delta % 64;
        if self.bits[word_idx] & (1u64 << bit_idx) != 0 {
            return Err("duplicate nonce");
        }
        self.bits[word_idx] |= 1u64 << bit_idx;
        Ok(())
    }

    /// Shift the entire bitfield right by `n` bits (drops the lowest `n` bits,
    /// advancing the window). This is the key operation — O(16) with no allocation.
    ///
    /// Forward iteration is safe because we always read from src >= i,
    /// so source words are never overwritten before they are read.
    #[inline]
    fn shift_right(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        if n >= WINDOW_BITS {
            self.bits = [0u64; WINDOW_U64S];
            return;
        }

        let word_shift = n / 64;
        let bit_shift = n % 64;

        if bit_shift == 0 {
            // Pure word-aligned shift — just move words down
            for i in 0..WINDOW_U64S {
                let src = i + word_shift;
                self.bits[i] = if src < WINDOW_U64S { self.bits[src] } else { 0 };
            }
        } else {
            // Cross-word bit shift
            for i in 0..WINDOW_U64S {
                let src = i + word_shift;
                let lo = if src < WINDOW_U64S { self.bits[src] } else { 0 };
                let hi = if src + 1 < WINDOW_U64S {
                    self.bits[src + 1]
                } else {
                    0
                };
                self.bits[i] = (lo >> bit_shift) | (hi << (64 - bit_shift));
            }
        }
    }
}

// ============================================================================
// Helper: generate deterministic tx hashes
// ============================================================================

fn make_tx_hash(seed: u64) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash[..8].copy_from_slice(&seed.to_le_bytes());
    hash[8..16].copy_from_slice(&seed.wrapping_mul(0x517cc1b727220a95).to_le_bytes());
    hash[16..24].copy_from_slice(&seed.wrapping_mul(0x6c62272e07bb0142).to_le_bytes());
    hash[24..32].copy_from_slice(&seed.wrapping_mul(0x9e3779b97f4a7c15).to_le_bytes());
    hash
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("sequential_1000");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("nonce", |b| {
        b.iter(|| {
            let mut state = NonceState::default();
            for i in 0u64..1000 {
                assert!(state.check(black_box(i)));
                state.mark(black_box(i));
            }
            black_box(&state);
        });
    });

    group.bench_function("generation", |b| {
        b.iter(|| {
            let mut state = GenerationState::default();
            for i in 0u64..1000 {
                let hash = make_tx_hash(i);
                assert!(state.check(black_box(i), black_box(hash)));
                state.mark(black_box(i), black_box(hash));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v1", |b| {
        b.iter(|| {
            let mut state = WindowV1State::default();
            for i in 0u64..1000 {
                assert!(state.check(black_box(i)));
                state.mark(black_box(i));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v2", |b| {
        b.iter(|| {
            let mut state = WindowV2State::default();
            for i in 0u64..1000 {
                assert!(state.check(black_box(i)).is_ok());
                state.mark(black_box(i)).unwrap();
            }
            black_box(&state);
        });
    });

    group.finish();
}

fn bench_random_in_window(c: &mut Criterion) {
    // Use 700 random nonces within [0, 700). This stays below v2's ADVANCE_THRESHOLD
    // (768) and well within v1's window (1000), so no sliding invalidates earlier nonces.
    let count = 700u64;
    let mut group = c.benchmark_group("random_in_window_700");
    group.throughput(Throughput::Elements(count));

    let mut rng = StdRng::seed_from_u64(42);
    let mut nonces: Vec<u64> = (0..count).collect();
    // Fisher-Yates shuffle
    for i in (1..nonces.len()).rev() {
        let j = rng.gen_range(0..=i);
        nonces.swap(i, j);
    }

    // Nonce approach can't do random — skip it (it requires sequential)

    group.bench_function("generation", |b| {
        let nonces = nonces.clone();
        b.iter(|| {
            let mut state = GenerationState::default();
            for &n in &nonces {
                let hash = make_tx_hash(n);
                assert!(state.check(black_box(n), black_box(hash)));
                state.mark(black_box(n), black_box(hash));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v1", |b| {
        let nonces = nonces.clone();
        b.iter(|| {
            let mut state = WindowV1State::default();
            for &n in &nonces {
                assert!(state.check(black_box(n)));
                state.mark(black_box(n));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v2", |b| {
        let nonces = nonces.clone();
        b.iter(|| {
            let mut state = WindowV2State::default();
            for &n in &nonces {
                assert!(state.check(black_box(n)).is_ok());
                state.mark(black_box(n)).unwrap();
            }
            black_box(&state);
        });
    });

    group.finish();
}

fn bench_sparse_advance(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparse_advance_100");
    group.throughput(Throughput::Elements(100));

    // Nonces spaced 100 apart — forces window sliding on every operation
    let nonces: Vec<u64> = (0..100).map(|i| i * 100).collect();

    group.bench_function("generation", |b| {
        let nonces = nonces.clone();
        b.iter(|| {
            let mut state = GenerationState::default();
            for &n in &nonces {
                let hash = make_tx_hash(n);
                assert!(state.check(black_box(n), black_box(hash)));
                state.mark(black_box(n), black_box(hash));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v1", |b| {
        let nonces = nonces.clone();
        b.iter(|| {
            let mut state = WindowV1State::default();
            for &n in &nonces {
                assert!(state.check(black_box(n)));
                state.mark(black_box(n));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v2", |b| {
        let nonces = nonces.clone();
        b.iter(|| {
            let mut state = WindowV2State::default();
            for &n in &nonces {
                assert!(state.check(black_box(n)).is_ok());
                state.mark(black_box(n)).unwrap();
            }
            black_box(&state);
        });
    });

    group.finish();
}

fn bench_burst_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("burst_cold_100");
    group.throughput(Throughput::Elements(100));

    group.bench_function("nonce", |b| {
        b.iter(|| {
            let mut state = NonceState::default();
            for i in 0u64..100 {
                assert!(state.check(black_box(i)));
                state.mark(black_box(i));
            }
            black_box(&state);
        });
    });

    group.bench_function("generation", |b| {
        b.iter(|| {
            let mut state = GenerationState::default();
            for i in 0u64..100 {
                let hash = make_tx_hash(i);
                assert!(state.check(black_box(i), black_box(hash)));
                state.mark(black_box(i), black_box(hash));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v1", |b| {
        b.iter(|| {
            let mut state = WindowV1State::default();
            for i in 0u64..100 {
                assert!(state.check(black_box(i)));
                state.mark(black_box(i));
            }
            black_box(&state);
        });
    });

    group.bench_function("window_v2", |b| {
        b.iter(|| {
            let mut state = WindowV2State::default();
            for i in 0u64..100 {
                assert!(state.check(black_box(i)).is_ok());
                state.mark(black_box(i)).unwrap();
            }
            black_box(&state);
        });
    });

    group.finish();
}

fn bench_serde_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("serde_roundtrip");

    // Prepare states at realistic fullness
    for &tx_count in &[100u64, 500, 1000] {
        // Nonce state
        let nonce_state = NonceState {
            next_nonce: tx_count,
        };
        let nonce_bytes = borsh::to_vec(&nonce_state).unwrap();

        group.bench_with_input(
            BenchmarkId::new("nonce", tx_count),
            &nonce_bytes,
            |b, bytes| {
                b.iter(|| {
                    let state: NonceState =
                        BorshDeserialize::try_from_slice(black_box(bytes)).unwrap();
                    let out = borsh::to_vec(black_box(&state)).unwrap();
                    black_box(out);
                });
            },
        );

        // Generation state — fill with tx_count hashes spread across generations
        let mut gen_state = GenerationState::default();
        for i in 0..tx_count.min(MAX_STORED_TX_HASHES) {
            let generation = i % PAST_TRANSACTION_GENERATIONS;
            gen_state.mark(generation, make_tx_hash(i));
        }
        let gen_bytes = borsh::to_vec(&gen_state).unwrap();

        group.bench_with_input(
            BenchmarkId::new("generation", tx_count),
            &gen_bytes,
            |b, bytes| {
                b.iter(|| {
                    let state: GenerationState =
                        BorshDeserialize::try_from_slice(black_box(bytes)).unwrap();
                    let out = borsh::to_vec(black_box(&state)).unwrap();
                    black_box(out);
                });
            },
        );

        // Window v1 state
        let mut v1_state = WindowV1State::default();
        for i in 0..tx_count {
            v1_state.mark(i);
        }
        let v1_bytes = borsh::to_vec(&v1_state).unwrap();

        group.bench_with_input(
            BenchmarkId::new("window_v1", tx_count),
            &v1_bytes,
            |b, bytes| {
                b.iter(|| {
                    let state: WindowV1State =
                        BorshDeserialize::try_from_slice(black_box(bytes)).unwrap();
                    let out = borsh::to_vec(black_box(&state)).unwrap();
                    black_box(out);
                });
            },
        );

        // Window v2 state
        let mut v2_state = WindowV2State::default();
        for i in 0..tx_count.min(WINDOW_BITS as u64) {
            v2_state.mark(i).unwrap();
        }
        let v2_bytes = borsh::to_vec(&v2_state).unwrap();

        group.bench_with_input(
            BenchmarkId::new("window_v2", tx_count),
            &v2_bytes,
            |b, bytes| {
                b.iter(|| {
                    let state: WindowV2State =
                        BorshDeserialize::try_from_slice(black_box(bytes)).unwrap();
                    let out = borsh::to_vec(black_box(&state)).unwrap();
                    black_box(out);
                });
            },
        );
    }

    group.finish();
}

fn bench_check_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("check_only_1000");
    group.throughput(Throughput::Elements(1000));

    // Pre-populate states, then check 1000 *valid* nonces (that haven't been used)

    // Nonce: populated to 0, check nonces 0..1000 (only first succeeds, but we measure the check cost)
    let nonce_state = NonceState { next_nonce: 500 };
    group.bench_function("nonce", |b| {
        let state = nonce_state.clone();
        b.iter(|| {
            for i in 500u64..1500 {
                black_box(state.check(black_box(i)));
            }
        });
    });

    // Generation: populated with some entries, check new valid nonces
    let mut gen_state = GenerationState::default();
    for i in 0..100u64 {
        gen_state.mark(i, make_tx_hash(i));
    }
    group.bench_function("generation", |b| {
        let state = gen_state.clone();
        b.iter(|| {
            for i in 0u64..1000 {
                let hash = make_tx_hash(i + 10000); // unique hashes not in state
                black_box(state.check(black_box(i % 100), black_box(hash)));
            }
        });
    });

    // Window v1: populated with some entries
    let mut v1_state = WindowV1State::default();
    for i in 0..500u64 {
        v1_state.mark(i);
    }
    group.bench_function("window_v1", |b| {
        let state = v1_state.clone();
        b.iter(|| {
            for i in 500u64..1500 {
                black_box(state.check(black_box(i)));
            }
        });
    });

    // Window v2: populated with some entries
    let mut v2_state = WindowV2State::default();
    for i in 0..500u64 {
        v2_state.mark(i).unwrap();
    }
    group.bench_function("window_v2", |b| {
        let state = v2_state.clone();
        b.iter(|| {
            for i in 500u64..1500 {
                let _ = black_box(state.check(black_box(i)));
            }
        });
    });

    group.finish();
}

/// Not a timed benchmark — prints a storage comparison table.
fn bench_storage_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_size_bytes");

    // We abuse criterion slightly to record storage sizes as "throughput"
    for &tx_count in &[0u64, 10, 100, 500, 1000, 1700] {
        let nonce_state = NonceState {
            next_nonce: tx_count,
        };
        let nonce_size = borsh::to_vec(&nonce_state).unwrap().len();

        let mut gen_state = GenerationState::default();
        for i in 0..tx_count.min(MAX_STORED_TX_HASHES) {
            let gen = i % PAST_TRANSACTION_GENERATIONS;
            gen_state.mark(gen, make_tx_hash(i));
        }
        let gen_size = borsh::to_vec(&gen_state).unwrap().len();

        let mut v1_state = WindowV1State::default();
        for i in 0..tx_count {
            v1_state.mark(i);
        }
        let v1_size = borsh::to_vec(&v1_state).unwrap().len();

        let mut v2_state = WindowV2State::default();
        for i in 0..tx_count.min(WINDOW_BITS as u64) {
            v2_state.mark(i).unwrap();
        }
        let v2_size = borsh::to_vec(&v2_state).unwrap().len();

        // Bench a no-op just to record the label, but print sizes to stderr
        group.bench_function(BenchmarkId::new("nonce", tx_count), |b| {
            b.iter(|| black_box(nonce_size))
        });
        group.bench_function(BenchmarkId::new("generation", tx_count), |b| {
            b.iter(|| black_box(gen_size))
        });
        group.bench_function(BenchmarkId::new("window_v1", tx_count), |b| {
            b.iter(|| black_box(v1_size))
        });
        group.bench_function(BenchmarkId::new("window_v2", tx_count), |b| {
            b.iter(|| black_box(v2_size))
        });

        eprintln!(
            "| {:>5} txs | nonce: {:>6} B | generation: {:>6} B | window_v1: {:>6} B | window_v2: {:>6} B |",
            tx_count, nonce_size, gen_size, v1_size, v2_size
        );
    }

    group.finish();
}

/// Single-operation microbenchmark: one check + one mark on pre-populated state.
/// This is the most representative of the per-transaction cost in the sequencer.
/// Uses iter_batched to reset state between samples, preventing state drift.
fn bench_single_op(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_check_and_mark");

    group.bench_function("nonce", |b| {
        b.iter_batched(
            || (NonceState { next_nonce: 500 }, 500u64),
            |(mut state, nonce)| {
                assert!(state.check(black_box(nonce)));
                state.mark(black_box(nonce));
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    let gen_state_template = {
        let mut s = GenerationState::default();
        for i in 0..500u64 {
            s.mark(i % 100, make_tx_hash(i));
        }
        s
    };
    group.bench_function("generation", |b| {
        b.iter_batched(
            || (gen_state_template.clone(), 500u64),
            |(mut state, nonce)| {
                let hash = make_tx_hash(nonce);
                assert!(state.check(black_box(nonce), black_box(hash)));
                state.mark(black_box(nonce), black_box(hash));
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    let v1_state_template = {
        let mut s = WindowV1State::default();
        for i in 0..500u64 {
            s.mark(i);
        }
        s
    };
    group.bench_function("window_v1", |b| {
        b.iter_batched(
            || (v1_state_template.clone(), 500u64),
            |(mut state, nonce)| {
                assert!(state.check(black_box(nonce)));
                state.mark(black_box(nonce));
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    let v2_state_template = {
        let mut s = WindowV2State::default();
        for i in 0..500u64 {
            s.mark(i).unwrap();
        }
        s
    };
    group.bench_function("window_v2", |b| {
        b.iter_batched(
            || (v2_state_template.clone(), 500u64),
            |(mut state, nonce)| {
                assert!(state.check(black_box(nonce)).is_ok());
                state.mark(black_box(nonce)).unwrap();
                black_box(&state);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

/// End-to-end: deserialize → check → mark → serialize (the full per-tx hot path).
/// Uses iter_batched to avoid state drift between samples.
fn bench_full_hot_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_hot_path_deser_check_mark_ser");

    // Nonce
    let nonce_bytes_template = borsh::to_vec(&NonceState { next_nonce: 500 }).unwrap();
    group.bench_function("nonce", |b| {
        b.iter_batched(
            || (nonce_bytes_template.clone(), 500u64),
            |(bytes, nonce)| {
                let mut state: NonceState =
                    BorshDeserialize::try_from_slice(black_box(&bytes)).unwrap();
                assert!(state.check(nonce));
                state.mark(nonce);
                let out = borsh::to_vec(&state).unwrap();
                black_box(out);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Generation
    let gen_bytes_template = {
        let mut s = GenerationState::default();
        for i in 0..500u64 {
            s.mark(i % 100, make_tx_hash(i));
        }
        borsh::to_vec(&s).unwrap()
    };
    group.bench_function("generation", |b| {
        b.iter_batched(
            || (gen_bytes_template.clone(), 500u64),
            |(bytes, nonce)| {
                let mut state: GenerationState =
                    BorshDeserialize::try_from_slice(black_box(&bytes)).unwrap();
                let hash = make_tx_hash(nonce);
                assert!(state.check(nonce, hash));
                state.mark(nonce, hash);
                let out = borsh::to_vec(&state).unwrap();
                black_box(out);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Window v1
    let v1_bytes_template = {
        let mut s = WindowV1State::default();
        for i in 0..500u64 {
            s.mark(i);
        }
        borsh::to_vec(&s).unwrap()
    };
    group.bench_function("window_v1", |b| {
        b.iter_batched(
            || (v1_bytes_template.clone(), 500u64),
            |(bytes, nonce)| {
                let mut state: WindowV1State =
                    BorshDeserialize::try_from_slice(black_box(&bytes)).unwrap();
                assert!(state.check(nonce));
                state.mark(nonce);
                let out = borsh::to_vec(&state).unwrap();
                black_box(out);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Window v2
    let v2_bytes_template = {
        let mut s = WindowV2State::default();
        for i in 0..500u64 {
            s.mark(i).unwrap();
        }
        borsh::to_vec(&s).unwrap()
    };
    group.bench_function("window_v2", |b| {
        b.iter_batched(
            || (v2_bytes_template.clone(), 500u64),
            |(bytes, nonce)| {
                let mut state: WindowV2State =
                    BorshDeserialize::try_from_slice(black_box(&bytes)).unwrap();
                assert!(state.check(nonce).is_ok());
                state.mark(nonce).unwrap();
                let out = borsh::to_vec(&state).unwrap();
                black_box(out);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(2))
        .sample_size(30);
    targets =
        bench_sequential,
        bench_random_in_window,
        bench_sparse_advance,
        bench_burst_cold,
        bench_serde_roundtrip,
        bench_check_only,
        bench_single_op,
        bench_full_hot_path,
        bench_storage_sizes,
}
criterion_main!(benches);

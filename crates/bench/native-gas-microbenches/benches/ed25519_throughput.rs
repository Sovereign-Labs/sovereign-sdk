//! Hardware ceiling for ed25519 signature verification throughput.
//!
//! Standalone on purpose: uses `ed25519-dalek` directly, with no SDK types, so the result is the
//! best case any tx-authentication pipeline could reach on this machine. Keys, messages and
//! signatures are pre-generated (not timed); each iteration verifies all of them:
//!   - `single_serial`: `verify_strict` on one thread, like the STF tx loop today.
//!   - `single_parallel`: `verify_strict` across rayon's global pool.
//!   - `batch_parallel/<chunk>`: `verify_batch` per chunk, chunks spread across rayon's pool.
//!
//! Criterion's `elem/s` is signatures verified per second, i.e. the TPS ceiling for
//! single-signature transactions.
//!
//! `verify_batch` is NOT a drop-in replacement for `verify_strict`: it skips the small-order
//! `R` / public-key rejection that `verify_strict` performs, so it accepts signatures production
//! rejects. Read the batch numbers as an upper bound, not as a consensus-compatible option.
//!
//! Run: `cargo bench -p native-gas-microbenches --bench ed25519_throughput`.
//! Pool size follows `RAYON_NUM_THREADS`. curve25519-dalek selects its SIMD backend at runtime.

use std::hint::black_box;
use std::time::Duration;

use criterion::{
    criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use ed25519_dalek::{verify_batch, Signature, Signer, SigningKey, VerifyingKey};
use rayon::prelude::*;

const NUM_SIGNATURES: usize = 65_536;
/// 256 B covers a bank transfer's signing payload (~150-170 B); 1 KiB and 4 KiB stand in for
/// heavier runtime calls.
const MESSAGE_SIZES: &[usize] = &[256, 1024, 4096];
const BATCH_CHUNK_SIZES: &[usize] = &[64, 256, 1024];

struct Corpus {
    messages: Vec<Vec<u8>>,
    signatures: Vec<Signature>,
    verifying_keys: Vec<VerifyingKey>,
}

impl Corpus {
    fn new(signing_keys: &[SigningKey], message_size: usize) -> Self {
        let messages: Vec<Vec<u8>> = (0..signing_keys.len())
            .map(|i| {
                (0..message_size)
                    .map(|j| (i.wrapping_mul(0x9E37) ^ j.wrapping_mul(0xAB)) as u8)
                    .collect()
            })
            .collect();
        let signatures = signing_keys
            .par_iter()
            .zip(messages.par_iter())
            .map(|(key, msg)| key.sign(msg))
            .collect();
        let verifying_keys = signing_keys.iter().map(SigningKey::verifying_key).collect();
        Self {
            messages,
            signatures,
            verifying_keys,
        }
    }

    fn verify_strict(&self, i: usize) -> bool {
        self.verifying_keys[i]
            .verify_strict(black_box(&self.messages[i]), &self.signatures[i])
            .is_ok()
    }
}

/// Distinct deterministic keys: one per simulated user, no RNG needed.
fn signing_keys() -> Vec<SigningKey> {
    (0..NUM_SIGNATURES as u64)
        .map(|i| {
            let mut seed = [0xA5u8; 32];
            seed[..8].copy_from_slice(&i.to_le_bytes());
            SigningKey::from_bytes(&seed)
        })
        .collect()
}

fn bench_ed25519_throughput(c: &mut Criterion) {
    eprintln!(
        "ed25519_throughput: {NUM_SIGNATURES} signatures per iteration, rayon pool = {} threads",
        rayon::current_num_threads()
    );
    let signing_keys = signing_keys();

    for &size in MESSAGE_SIZES {
        let corpus = Corpus::new(&signing_keys, size);
        let message_refs: Vec<&[u8]> = corpus.messages.iter().map(Vec::as_slice).collect();

        let mut group = c.benchmark_group(format!("ed25519_throughput/{size}B"));
        group.throughput(Throughput::Elements(NUM_SIGNATURES as u64));
        group.sampling_mode(SamplingMode::Flat);
        group.sample_size(10);

        // A single-threaded pass over the corpus takes seconds, so give it a longer window.
        group.measurement_time(Duration::from_secs(40));
        group.bench_function("single_serial", |b| {
            b.iter(|| assert!((0..NUM_SIGNATURES).all(|i| corpus.verify_strict(i))));
        });

        group.measurement_time(Duration::from_secs(10));
        group.bench_function("single_parallel", |b| {
            b.iter(|| {
                assert!((0..NUM_SIGNATURES)
                    .into_par_iter()
                    .all(|i| corpus.verify_strict(i)));
            });
        });

        for &chunk in BATCH_CHUNK_SIZES {
            group.bench_with_input(
                BenchmarkId::new("batch_parallel", chunk),
                &chunk,
                |b, &chunk| {
                    b.iter(|| {
                        assert!(message_refs
                            .par_chunks(chunk)
                            .zip(corpus.signatures.par_chunks(chunk))
                            .zip(corpus.verifying_keys.par_chunks(chunk))
                            .all(|((msgs, sigs), keys)| {
                                verify_batch(black_box(msgs), sigs, keys).is_ok()
                            }));
                    });
                },
            );
        }
        group.finish();
    }
}

criterion_group!(benches, bench_ed25519_throughput);
criterion_main!(benches);

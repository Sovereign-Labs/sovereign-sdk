//! Native calibration of the signature-verification gas constants.
//!
//! Mirrors the SP1 `ed25519` microbench's call site (`MeteredSignature::verify`)
//! and input sweep, but measures native wall-clock instead of prover gas. Fits
//! `ns = bias + per_byte * size`:
//!   - bias     -> DEFAULT_FIXED_GAS_TO_CHARGE_PER_SIGNATURE_VERIFICATION
//!   - per_byte -> DEFAULT_GAS_TO_CHARGE_PER_BYTE_SIGNATURE_VERIFICATION

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sov_gas_tools::report::report_size_sweep;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CryptoSpec, MeteredSignature, PrivateKey, Spec, UnlimitedGasMeter};

type MicrobenchSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;
type Crypto = <MicrobenchSpec as Spec>::CryptoSpec;
type PrivKey = <Crypto as CryptoSpec>::PrivateKey;
type Sig = <Crypto as CryptoSpec>::Signature;

const GROUP: &str = "ed25519";
// Mirror the SP1 ed25519 sweep so native and ZK calibrations are comparable.
const SIZES: &[u64] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

fn bench_ed25519(c: &mut Criterion) {
    let private_key = PrivKey::generate();
    let pub_key = private_key.pub_key();

    let mut group = c.benchmark_group(GROUP);
    for &size in SIZES {
        let msg: Vec<u8> = (0..size).map(|i| (i as u8).wrapping_mul(0xAB)).collect();
        let metered = MeteredSignature::<<MicrobenchSpec as Spec>::Gas, Sig>::new::<MicrobenchSpec>(
            private_key.sign(&msg),
        );
        group.bench_with_input(BenchmarkId::from_parameter(size), &msg, |b, msg| {
            let mut meter = UnlimitedGasMeter::<MicrobenchSpec>::default();
            b.iter(|| {
                metered
                    .verify(black_box(&pub_key), black_box(msg.as_slice()), &mut meter)
                    .expect("a freshly-signed message must verify");
            });
        });
    }
    group.finish();

    if let Err(e) = report_size_sweep(
        GROUP,
        SIZES,
        "DEFAULT_FIXED_GAS_TO_CHARGE_PER_SIGNATURE_VERIFICATION",
        "DEFAULT_GAS_TO_CHARGE_PER_BYTE_SIGNATURE_VERIFICATION",
    ) {
        eprintln!("warn: could not compute fit / constants suggestion: {e}");
    }
}

criterion_group!(benches, bench_ed25519);
criterion_main!(benches);

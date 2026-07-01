//! Native calibration of sha256 gas constants.
//!
//! Mirrors the SP1 `sha256` microbench's call site (`MeteredHasher::digest`) and
//! input sweep, but measures native wall-clock instead of prover gas. Fits
//! `ns = bias + per_byte * size`:
//!   - bias     -> GAS_TO_CHARGE_HASH_UPDATE
//!   - per_byte -> GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sov_gas_tools::report::report_size_sweep;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CryptoSpec, MeteredHasher, Spec, UnlimitedGasMeter};

type MicrobenchSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;
type Hasher = <<MicrobenchSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher;

const GROUP: &str = "sha256";
// Mirror the SP1 sha256 sweep so native and ZK calibrations are comparable.
const SIZES: &[u64] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

fn bench_sha256(c: &mut Criterion) {
    let mut group = c.benchmark_group(GROUP);
    for &size in SIZES {
        let buf: Vec<u8> = (0..size).map(|i| (i as u8).wrapping_mul(0xAB)).collect();
        group.bench_with_input(BenchmarkId::from_parameter(size), &buf, |b, buf| {
            let mut meter = UnlimitedGasMeter::<MicrobenchSpec>::default();
            b.iter(|| {
                black_box(
                    MeteredHasher::<UnlimitedGasMeter<MicrobenchSpec>, Hasher>::digest(
                        black_box(buf.as_slice()),
                        &mut meter,
                    )
                    .expect("UnlimitedGasMeter never errors"),
                )
            });
        });
    }
    group.finish();

    if let Err(e) = report_size_sweep(
        GROUP,
        SIZES,
        "GAS_TO_CHARGE_HASH_UPDATE",
        "GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE",
    ) {
        eprintln!("warn: could not compute fit / constants suggestion: {e}");
    }
}

criterion_group!(benches, bench_sha256);
criterion_main!(benches);

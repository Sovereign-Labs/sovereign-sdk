#![no_main]

sp1_zkvm::entrypoint!(main);

use core::hint::black_box;

use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::execution_mode::Zk;
use sov_modules_api::{CryptoSpec, MeteredHasher, Spec, UnlimitedGasMeter};

type MicrobenchSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Zk>;
type Hasher = <<MicrobenchSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher;

pub fn main() {
    let byte_len: u32 = sp1_zkvm::io::read();
    let iterations: u32 = sp1_zkvm::io::read();

    let buf: Vec<u8> = (0..byte_len)
        .map(|i| (i as u8).wrapping_mul(0xAB))
        .collect();

    let buf = black_box(buf);
    let mut meter = UnlimitedGasMeter::<MicrobenchSpec>::default();

    println!("cycle-tracker-report-start: hash_loop");
    let mut last = [0u8; 32];
    for _ in 0..iterations {
        // Using black_box actually makes a meaningful difference.
        // LLVM appears to perform some optimiziations on this code that removes
        // ~10% of cycles. We use black_box to achieve generated code that should be similar
        // to that in the STF
        last = black_box(
            MeteredHasher::<UnlimitedGasMeter<MicrobenchSpec>, Hasher>::digest(
                black_box(&buf),
                &mut meter,
            )
            .expect("UnlimitedGasMeter never errors"),
        );
    }
    println!("cycle-tracker-report-end: hash_loop");

    sp1_zkvm::io::commit(&last);
}

#![no_main]

sp1_zkvm::entrypoint!(main);

use core::hint::black_box;

use sov_rollup_interface::crypto::Signature;
use sov_sp1_adapter::crypto::{SP1PublicKey, SP1Signature};

pub fn main() {
    let pubkey_bytes = sp1_zkvm::io::read_vec();
    let sig_bytes = sp1_zkvm::io::read_vec();
    let msg = sp1_zkvm::io::read_vec();
    let iterations: u32 = sp1_zkvm::io::read();

    let pubkey = SP1PublicKey::try_from(pubkey_bytes).expect("valid pubkey");
    let sig = SP1Signature::try_from(sig_bytes.as_slice()).expect("valid signature");
    let msg = black_box(msg);

    println!("cycle-tracker-report-start: verify_loop");
    for _ in 0..iterations {
        // black_box to prevent LLVM from hoisting the verify out of the loop
        // or eliding work — same insurance pattern as guest-sha256.
        let result = sig.verify(black_box(&pubkey), black_box(&msg));
        black_box(result.expect("verification should succeed"));
    }
    println!("cycle-tracker-report-end: verify_loop");

    sp1_zkvm::io::commit(&[0u8; 32]);
}

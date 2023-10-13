#![no_main]
//! This binary implements the verification logic for the rollup. This is the code that runs inside
//! of the zkvm in order to generate proofs for the rollup.
use template_stf::{zk_stf, RollupVerifier};

use sov_rollup_interface::mocks::MockDaVerifier;
use sov_risc0_adapter::guest::Risc0Guest;
risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();

    let mut stf_verifier =
        RollupVerifier::new(zk_stf(), MockDaVerifier {});
    
    stf_verifier
        .run_block(guest)
        .expect("Prover must be honest");
}

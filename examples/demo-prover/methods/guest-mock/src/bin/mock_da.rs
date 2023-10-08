#![no_main]

use demo_stf::{AppVerifier, ZkApp};
use sov_risc0_adapter::guest::Risc0Guest;

use sov_rollup_interface::mocks::MockDaVerifier;

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();

    let mut stf_verifier =
        AppVerifier::new(ZkApp::<Risc0Guest, _>::default().stf, MockDaVerifier {});

    stf_verifier
        .run_block(guest)
        .expect("Prover must be honest");
}

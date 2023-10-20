#![no_main]
//! This binary implements the verification logic for the rollup. This is the code that runs inside
//! of the zkvm in order to generate proofs for the rollup.

use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_stf_template::AppTemplate;
use sov_risc0_adapter::guest::Risc0Guest;
use sov_rollup_interface::mocks::MockDaVerifier;
use sov_state::ZkStorage;
use stf_starter::runtime::Runtime;
use stf_starter::AppVerifier;

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();
    let storage = ZkStorage::new();
    let app: AppTemplate<ZkDefaultContext, _, _, Runtime<_, _>> = AppTemplate::new(storage);

    let mut stf_verifier = AppVerifier::new(app, MockDaVerifier {});

    stf_verifier
        .run_block(guest)
        .expect("Prover must be honest");
}

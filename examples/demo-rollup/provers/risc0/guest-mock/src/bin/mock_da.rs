#![no_main]

use demo_stf::app::create_zk_app_template;
use demo_stf::AppVerifier;
use sov_risc0_adapter::guest::Risc0Guest;

use sov_rollup_interface::mocks::MockDaVerifier;

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();

    let mut stf_verifier =
        AppVerifier::new(create_zk_app_template::<Risc0Guest, _>(), MockDaVerifier {});
    
    stf_verifier
        .run_block(guest)
        .expect("Prover must be honest");
}

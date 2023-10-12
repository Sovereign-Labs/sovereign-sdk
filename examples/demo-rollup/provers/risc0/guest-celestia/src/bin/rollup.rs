// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]

use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use sov_celestia_adapter::types::Namespace;
use sov_celestia_adapter::verifier::CelestiaVerifier;
use sov_risc0_adapter::guest::Risc0Guest;
use demo_stf::{create_zk_app_template, AppVerifier};

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_NAMESPACE: Namespace = Namespace::const_v0(ROLLUP_NAMESPACE_RAW);

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();
    let mut stf_verifier = AppVerifier::new(
        create_zk_app_template::<Risc0Guest, _>(),
        CelestiaVerifier {
            rollup_namespace: ROLLUP_NAMESPACE,
        },
    );
    stf_verifier
        .run_block(guest)
        .expect("Prover must be honest");
}

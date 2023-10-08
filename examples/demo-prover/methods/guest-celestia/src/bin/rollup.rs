// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]

use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use demo_stf::{AppVerifier, ZkApp};
use sov_celestia_adapter::types::NamespaceId;
use sov_celestia_adapter::verifier::CelestiaVerifier;
use sov_risc0_adapter::guest::Risc0Guest;

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();
    let mut stf_verifier = AppVerifier::new(
        ZkApp::<Risc0Guest, _>::default().stf,
        CelestiaVerifier {
            rollup_namespace: ROLLUP_NAMESPACE,
        },
    );
    stf_verifier
        .run_block(guest)
        .expect("Prover must be honest");
}

// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]

use const_rollup_config::{ROLLUP_BATCH_NAMESPACE_RAW, ROLLUP_PROOF_NAMESPACE_RAW};
use demo_stf::runtime::Runtime;
use demo_stf::StfVerifier;
use sov_address::MultiAddressEvm;
use sov_celestia_adapter::types::Namespace;
use sov_celestia_adapter::verifier::{CelestiaSpec, CelestiaVerifier};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Zk;
use sov_modules_stf_blueprint::StfBlueprint;
use sov_risc0_adapter::guest::Risc0Guest;
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::da::DaVerifier;
use sov_state::ZkStorage;

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_BATCH_NAMESPACE: Namespace = Namespace::const_v0(ROLLUP_BATCH_NAMESPACE_RAW);
const ROLLUP_PROOF_NAMESPACE: Namespace = Namespace::const_v0(ROLLUP_PROOF_NAMESPACE_RAW);

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();
    let storage = ZkStorage::new();
    let stf: StfBlueprint<
        ConfigurableSpec<CelestiaSpec, Risc0, MockZkvm, MultiAddressEvm, Zk>,
        Runtime<_>,
    > = StfBlueprint::new();

    let rollup_params = sov_celestia_adapter::verifier::RollupParams {
        rollup_batch_namespace: ROLLUP_BATCH_NAMESPACE,
        rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
    };

    let stf_verifier = StfVerifier::<_, _, _, _, _>::new(stf, CelestiaVerifier::new(rollup_params));
    stf_verifier
        .run_block(guest, storage)
        .expect("Prover must be honest");
}

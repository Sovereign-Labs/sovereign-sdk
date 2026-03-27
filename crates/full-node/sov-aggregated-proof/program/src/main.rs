#![no_main]

sp1_zkvm::entrypoint!(main);

use demo_stf::MultiAddressEvmSolana;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::{execution_mode::Zk, CodeCommitmentHash, Spec, Storage};
use sov_rollup_interface::zk::aggregated_proof::circuit::run_aggregation_program;
use sov_sp1_adapter::SP1;
use sov_sp1_adapter::{guest::SP1Guest, SP1Verifier};

include!(concat!(env!("OUT_DIR"), "/inner_vk_hash.rs"));

type ProgramSpec = ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvmSolana, Zk>;

pub fn main() {
    let guest = SP1Guest::new();

    let inner = CodeCommitmentHash::from_u32_array(INNER_VKEY_HASH);
    run_aggregation_program::<
        <ProgramSpec as Spec>::Address,
        MockDaSpec,
        <<ProgramSpec as Spec>::Storage as Storage>::Root,
        SP1Verifier,
        SP1Guest,
    >(inner, guest);
}

#![no_main]

sp1_zkvm::entrypoint!(main);

use demo_stf::runtime::Runtime;
use demo_stf::StfVerifier;
use sov_address::MultiAddressEvm;
use sov_mock_da::{MockDaSpec, MockDaVerifier};
pub use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Zk;
use sov_modules_stf_blueprint::StfBlueprint;
use sov_sp1_adapter::guest::SP1Guest;
use sov_sp1_adapter::SP1;
use sov_state::ZkStorage;

#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
pub fn main() {
    let guest = SP1Guest::new();
    let storage = ZkStorage::new();

    let stf: StfBlueprint<
        ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvm, Zk>,
        Runtime<_>,
    > = StfBlueprint::new();

    let stf_verifier = StfVerifier::<_, _, _, _, _>::new(stf, MockDaVerifier {});

    stf_verifier
        .run_block(guest, storage)
        .expect("Prover must be honest");
}

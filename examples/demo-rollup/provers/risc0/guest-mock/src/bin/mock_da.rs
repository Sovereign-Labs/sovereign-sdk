#![no_main]
use demo_stf::runtime::Runtime;
use demo_stf::{MultiAddressEvmSolana, StfVerifier};
use sov_mock_da::{MockDaSpec, MockDaVerifier};
pub use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Zk;
use sov_modules_stf_blueprint::StfBlueprint;
use sov_risc0_adapter::guest::Risc0Guest;
use sov_risc0_adapter::Risc0;
use sov_state::nomt::zk_storage::NomtVerifierStorage;
use sov_state::DefaultStorageSpec;

type NomtStorage = NomtVerifierStorage<DefaultStorageSpec<sha2::Sha256>>;

risc0_zkvm::guest::entry!(main);

#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
fn cycles_per_block() {
    let guest = Risc0Guest::new();
    let storage = NomtStorage::new();

    let stf: StfBlueprint<
        ConfigurableSpec<
            MockDaSpec,
            Risc0,
            MockZkvm,
            MultiAddressEvmSolana,
            Zk,
            sov_risc0_adapter::Risc0CryptoSpec,
            NomtStorage,
        >,
        Runtime<_>,
    > = StfBlueprint::new();

    let stf_verifier = StfVerifier::new(stf, MockDaVerifier {});

    stf_verifier
        .run_block(guest, storage)
        .expect("Prover must be honest");
}

pub fn main() {
    cycles_per_block();
}

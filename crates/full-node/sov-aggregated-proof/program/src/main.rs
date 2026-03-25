#![no_main]

mod aggregation;

sp1_zkvm::entrypoint!(main);

use crate::aggregation::run_aggregation_program;
use demo_stf::MultiAddressEvmSolana;
use sov_aggregated_proof_shared::AggregatedProofWitness;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::{execution_mode::Zk, CodeCommitmentHash};
use sov_sp1_adapter::SP1Verifier;
use sov_sp1_adapter::SP1;

include!(concat!(env!("OUT_DIR"), "/inner_vk_hash.rs"));

type ProgramSpec = ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvmSolana, Zk>;

pub fn main() {
    let witness = sp1_zkvm::io::read::<AggregatedProofWitness<MockDaSpec>>();

    let inner = CodeCommitmentHash(INNER_VKEY_HASH);
    run_aggregation_program::<ProgramSpec, MockDaSpec, SP1Verifier>(witness, inner);
}

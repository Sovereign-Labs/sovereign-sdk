#![no_main]

sp1_zkvm::entrypoint!(main);

use demo_stf::MultiAddressEvmSolana;
use sha2::{Digest, Sha256};
use sov_aggregated_proof_shared::DeferredProofInput;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::da::BlockHeaderTrait;
use sov_modules_api::execution_mode::Zk;
use sov_modules_api::Spec;
use sov_modules_api::StateTransitionPublicData;
use sov_modules_api::Storage;
use sov_sp1_adapter::SP1;

type S = ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvmSolana, Zk>;

pub fn main() {
    let proof_inputs = sp1_zkvm::io::read::<Vec<DeferredProofInput>>();

    for (index, proof_input) in proof_inputs.iter().enumerate() {
        let stf_public_data: StateTransitionPublicData<
            <S as Spec>::Address,
            MockDaSpec,
            <<S as Spec>::Storage as Storage>::Root,
        > = bincode::deserialize(proof_input.public_values.as_slice()).unwrap();

        let da_block_header = &proof_input.da_block_header;
        assert_eq!(da_block_header.hash(), stf_public_data.slot_hash);

        println!("[guest] verifying proof {index}");
        let public_values_digest: [u8; 32] = Sha256::digest(&proof_input.public_values).into();
        sp1_zkvm::lib::verify::verify_sp1_proof(&proof_input.vkey_hash, &public_values_digest);
        println!("[guest] verified proof {index}");
    }
}

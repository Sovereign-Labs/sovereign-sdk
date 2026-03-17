#![no_main]

sp1_zkvm::entrypoint!(main);

use demo_stf::MultiAddressEvmSolana;
use sha2::{Digest, Sha256};
use sov_aggregated_proof_shared::{AggregatedProofWitness, DeferredProofInput};
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::da::BlockHeaderTrait;
use sov_modules_api::execution_mode::Zk;
use sov_modules_api::DaSpec;
use sov_modules_api::Spec;
use sov_modules_api::StateTransitionPublicData;
use sov_modules_api::Storage;
use sov_sp1_adapter::SP1;

type S = ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvmSolana, Zk>;

type StPubData<S: Spec, Da: DaSpec> =
    StateTransitionPublicData<<S as Spec>::Address, Da, <<S as Spec>::Storage as Storage>::Root>;

pub fn main() {
    let witness = sp1_zkvm::io::read::<AggregatedProofWitness<MockDaSpec>>();
    let proof_inputs = witness.proof_inputs;
    let vkey_hash = witness.vkey_hash;

    verify::<S, MockDaSpec>(proof_inputs, vkey_hash);
}

fn verify<S: Spec, Da: DaSpec>(proof_inputs: Vec<DeferredProofInput<Da>>, vkey_hash: [u32; 8]) {
    assert!(
        !proof_inputs.is_empty(),
        "Aggregated proof must contain at least one proof input"
    );

    // `None` means no predecessor to check against (first iteration).
    let mut expected_prev_hash = None;
    let mut expected_state_root = None;

    for (index, proof_input) in proof_inputs.iter().enumerate() {
        let stf_public_data =
            deserialize_pub_data::<S, Da>(proof_input.public_values.as_slice(), index);

        {
            let da_block_header = &proof_input.da_block_header;
            let current_block_hash = da_block_header.hash();

            // Check that DA blocks form a chain.
            if let Some(expected_prev_hash) = &expected_prev_hash {
                assert_eq!(
                    expected_prev_hash,
                    &da_block_header.prev_hash(),
                    "DA block chain broken at index {index}: prev_hash mismatch"
                );
            }

            // Check that the slot hash from the public input matches the current block hash.
            assert_eq!(
                current_block_hash, stf_public_data.slot_hash,
                "Slot hash mismatch at index {index}: DA block header hash doesn't match public data"
            );
            expected_prev_hash = Some(current_block_hash);
        }

        // Check that state roots are sequentially related by the state transition.
        {
            if let Some(expected_state_root) = &expected_state_root {
                assert_eq!(
                    expected_state_root, &stf_public_data.initial_state_root,
                    "State root discontinuity at index {index}: previous final_state_root != current initial_state_root"
                );
            }

            verify_sp1_proof(proof_input, vkey_hash);
            expected_state_root = Some(stf_public_data.final_state_root.clone());
        }
    }
}

fn verify_sp1_proof<Da: DaSpec>(proof_input: &DeferredProofInput<Da>, vkey_hash: [u32; 8]) {
    let public_values_digest: [u8; 32] = Sha256::digest(&proof_input.public_values).into();
    sp1_zkvm::lib::verify::verify_sp1_proof(&vkey_hash, &public_values_digest);
}

fn deserialize_pub_data<S: Spec, Da: DaSpec>(data: &[u8], index: usize) -> StPubData<S, Da> {
    bincode::deserialize(data).unwrap_or_else(|error| {
        panic!("Failed to deserialize public values from proof input {index}: {error}")
    })
}

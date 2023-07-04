// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]

use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use demo_stf::app::ZkAppRunner;
use demo_stf::ArrayWitness;
use jupiter::types::NamespaceId;
use jupiter::verifier::{CelestiaSpec, CelestiaVerifier};
use jupiter::{BlobWithSender, CelestiaHeader};
use risc0_adapter::guest::Risc0Guest;
use risc0_zkvm::guest::env;
use sov_rollup_interface::crypto::NoOpHasher;
use sov_rollup_interface::da::{DaSpec, DaVerifier};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::{StateTransitionFunction, ZkConfig};
use sov_rollup_interface::zk::traits::{StateTransition, ZkvmGuest};

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

risc0_zkvm::guest::entry!(main);
// steps:
//  0. Read tx list and proofs
//  1. Call verify_relevant_tx_list()
//  2. Call begin_slot()
//  3. Decode each batch.
//  4. Call apply_batch
//  5. Call end_slot
//  6. Output (Da hash, start_root, end_root, event_root)
pub fn main() {
    env::write(&"Start guest\n");
    let guest = Risc0Guest;

    let prev_state_root_hash: [u8; 32] = guest.read_from_host();
    env::write(&"Prev root hash read\n");
    // Step 1: read tx list
    let header: CelestiaHeader = guest.read_from_host();
    env::write(&"header read\n");
    let inclusion_proof: <CelestiaSpec as DaSpec>::InclusionMultiProof = guest.read_from_host();
    env::write(&"inclusion proof read\n");
    let completeness_proof: <CelestiaSpec as DaSpec>::CompletenessProof = guest.read_from_host();
    env::write(&"completeness proof read\n");
    let mut blobs: Vec<BlobWithSender> = guest.read_from_host();
    env::write(&"txs read\n");

    // Step 2: Apply blobs
    let mut demo_runner = <ZkAppRunner<Risc0Guest> as StateTransitionRunner<
        ZkConfig,
        Risc0Guest,
    >>::new(prev_state_root_hash);
    let demo = demo_runner.inner_mut();

    let witness: ArrayWitness = guest.read_from_host();
    env::write(&"Witness read\n");

    demo.begin_slot(witness);
    env::write(&"Slot has begun\n");
    for blob in &mut blobs {
        demo.apply_blob(blob, None);
        env::write(&"Blob applied\n");
    }
    let (state_root, _) = demo.end_slot();
    env::write(&"Slot has ended\n");

    // Step 3: Verify tx list
    let verifier = CelestiaVerifier::new(jupiter::verifier::RollupParams {
        namespace: ROLLUP_NAMESPACE,
    });
    let validity_condition = verifier
        .verify_relevant_tx_list::<NoOpHasher>(&header, &blobs, inclusion_proof, completeness_proof)
        .expect("Transaction list must be correct");
    env::write(&"Relevant txs verified\n");

    let output = StateTransition {
        initial_state_root: prev_state_root_hash,
        final_state_root: state_root.0,
        validity_condition,
    };
    env::commit(&output);
    env::write(&"new state root committed\n");
}

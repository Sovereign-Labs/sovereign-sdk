// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]
use demo_stf::app::ZkAppRunner;
use demo_stf::ArrayWitness;
use presence::spec::{DaLayerSpec};
use presence::spec::header::AvailHeader;
use presence::verifier::Verifier;
use log::info;
use risc0_adapter::guest::Risc0Guest;
use risc0_zkvm::guest::env;
use sov_rollup_interface::crypto::NoOpHasher;
use sov_rollup_interface::da::{DaSpec, DaVerifier};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::{StateTransitionFunction, ZkConfig};
use sov_rollup_interface::zk::{StateTransition, ValidityCondition, ZkvmGuest};

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
    // TODO: Remove this
    // info!("Should not be printed from guest");
    let guest = Risc0Guest;
    let verifier = presence::verifier::Verifier {};
    
    // Step 1: read tx list
    let header: <DaLayerSpec as DaSpec>::BlockHeader = guest.read_from_host();
    env::write(&"header read\n");
    let txs: Vec<<DaLayerSpec as DaSpec>::BlobTransaction> = guest.read_from_host();
    env::write(&"txs read\n");
    let inclusion_proof: <DaLayerSpec as DaSpec>::InclusionMultiProof = guest.read_from_host();
    let completeness_proof: <DaLayerSpec as DaSpec>::CompletenessProof = guest.read_from_host();

    // Step 2: Verify tx list
    let validity_condition = verifier
        .verify_relevant_tx_list::<NoOpHasher>(&header, &txs, inclusion_proof, completeness_proof)
        .expect("Transaction list must be correct");
    env::write(&"Relevant txs verified\n");

    state_transition(&guest, txs, validity_condition);
}

fn state_transition(
    guest: &Risc0Guest,
    batches: Vec<<DaLayerSpec as DaSpec>::BlobTransaction>,
    validity_condition: impl ValidityCondition,
) {
    let prev_state_root_hash: [u8; 32] = guest.read_from_host();
    env::write(&"Prev root hash read\n");

    let mut demo_runner = <ZkAppRunner<Risc0Guest> as StateTransitionRunner<
        ZkConfig,
        Risc0Guest,
    >>::new(prev_state_root_hash);

    let demo = demo_runner.inner_mut();

    let witness: ArrayWitness = guest.read_from_host();
    env::write(&"Witness read\n");

    demo.begin_slot(witness);
    env::write(&"Slot has begun\n");
    for mut batch in batches {
        demo.apply_blob(&mut batch, None);
        env::write(&"Blob applied\n");
    }
    let (state_root, _) = demo.end_slot();
    env::write(&"Slot has ended\n");
    let output = StateTransition {
        initial_state_root: prev_state_root_hash,
        final_state_root: state_root.0,
        validity_condition,
    };
    env::commit(&output);
    env::write(&"New state root committed\n");
}

#[test]
fn test() {}

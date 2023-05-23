// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]

use demo_stf::app::ZkAppRunner;
use demo_stf::ArrayWitness;
use jupiter::types::NamespaceId;
use jupiter::verifier::{CelestiaSpec, CelestiaVerifier};
use jupiter::{BlobWithSender, CelestiaHeader};
use risc0_adapter::guest::Risc0Guest;
use risc0_zkvm::guest::env;
use sov_rollup_interface::da::{DaSpec, DaVerifier};
use sov_rollup_interface::stf::{StateTransitionFunction, StateTransitionRunner, ZkConfig};
use sov_rollup_interface::zk::traits::ZkvmGuest;

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

    let verifier = CelestiaVerifier::new(jupiter::verifier::RollupParams {
        namespace: NamespaceId([115, 111, 118, 45, 116, 101, 115, 116]),
    });
    // Step 1: read tx list
    let header: CelestiaHeader = guest.read_from_host();
    env::write(&"header read\n");
    let txs: Vec<BlobWithSender> = guest.read_from_host();
    env::write(&"txs read\n");
    let inclusion_proof: <CelestiaSpec as DaSpec>::InclusionMultiProof = guest.read_from_host();
    let completeness_proof: <CelestiaSpec as DaSpec>::CompletenessProof = guest.read_from_host();

    // Step 2: Verify tx list
    verifier
        .verify_relevant_tx_list(&header, &txs, inclusion_proof, completeness_proof)
        .expect("Transaction list must be correct");
    env::write(&"Relevant txs verified\n");

    state_transition(&guest, txs);
}

fn state_transition(guest: &Risc0Guest, batches: Vec<BlobWithSender>) {
    let prev_state_root_hash: [u8; 32] = guest.read_from_host();
    env::commit_slice(&prev_state_root_hash[..]);
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
    for batch in batches {
        demo.apply_blob(batch, None);
        env::write(&"Blob applied\n");
    }
    let (state_root, _, _) = demo.end_slot();
    env::write(&"Slot has ended\n");
    env::commit(&state_root);
    env::write(&"new state root committed\n");
}

#[test]
fn test() {}

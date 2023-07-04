// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]

use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use demo_stf::app::ZkAppRunner;
use demo_stf::ArrayWitness;
use jupiter::types::NamespaceId;
use jupiter::verifier::{CelestiaSpec, CelestiaVerifier, ChainValidityCondition};
use jupiter::{BlobWithSender, CelestiaHeader};
use log::info;
use risc0_adapter::guest::Risc0Guest;
use risc0_zkvm::guest::env;
// use sov_rollup_interface::crypto::NoOpHasher;
use sov_rollup_interface::da::{DaSpec, DaVerifier};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::{StateTransitionFunction, ZkConfig};
// use sov_rollup_interface::zk::traits::{StateTransition, ValidityCondition, ZkvmGuest};
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
    // TODO: Remove this
    info!("Should not be printed from guest");
    let guest = Risc0Guest;

    let _verifier = CelestiaVerifier::new(jupiter::verifier::RollupParams {
        namespace: ROLLUP_NAMESPACE,
    });
    // Step 1: read tx list
    let _header: CelestiaHeader = guest.read_from_host();
    env::write(&"header read\n");
    let txs: Vec<BlobWithSender> = guest.read_from_host();
    env::write(&"txs read\n");
    let _inclusion_proof: <CelestiaSpec as DaSpec>::InclusionMultiProof = guest.read_from_host();
    let _completeness_proof: <CelestiaSpec as DaSpec>::CompletenessProof = guest.read_from_host();

    // Step 2: Verify tx list
    // TODO: uncomment when https://github.com/Sovereign-Labs/sovereign-sdk/issues/456 is resolved
    // let validity_condition = verifier
    //     .verify_relevant_tx_list::<NoOpHasher>(&header, &txs, inclusion_proof, completeness_proof)
    //     .expect("Transaction list must be correct");
    // env::write(&"Relevant txs verified\n");
    // state_transition(&guest, txs, validity_condition);
    state_transition(&guest, txs);
}

fn state_transition(
    guest: &Risc0Guest,
    batches: Vec<BlobWithSender>,
    // validity_condition: impl ValidityCondition,
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
    let validity_condition = ChainValidityCondition {
        prev_hash: prev_state_root_hash,
        block_hash: [0; 32],
    };
    let output = StateTransition {
        initial_state_root: prev_state_root_hash,
        final_state_root: state_root.0,
        validity_condition,
    };
    env::commit(&output);
    env::write(&"new state root committed\n");
}

#[test]
fn test() {}

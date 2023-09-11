// TODO: Rename this file to change the name of this method from METHOD_NAME

#![no_main]

use std::str::FromStr;

use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::create_zk_app_template;
use demo_stf::ArrayWitness;
use risc0_zkvm::guest::env;
use sov_celestia_adapter::types::NamespaceId;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::{CelestiaSpec, CelestiaVerifier};
use sov_celestia_adapter::{BlobWithSender, CelestiaHeader};
use sov_risc0_adapter::guest::Risc0Guest;
use sov_rollup_interface::crypto::NoOpHasher;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec, DaVerifier};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::{StateTransition, ZkvmGuest};

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

    #[cfg(feature = "bench")]
    let start_cycles = env::get_cycle_count();
    let prev_state_root_hash: [u8; 32] = guest.read_from_host();
    env::write(&"Prev root hash read\n");
    // Step 1: read tx list
    let header: CelestiaHeader = guest.read_from_host();
    env::write(&"header has been read\n");
    let inclusion_proof: <CelestiaSpec as DaSpec>::InclusionMultiProof = guest.read_from_host();
    env::write(&"inclusion proof has been read\n");
    let completeness_proof: <CelestiaSpec as DaSpec>::CompletenessProof = guest.read_from_host();
    env::write(&"completeness proof has been read\n");
    let mut blobs: Vec<BlobWithSender> = guest.read_from_host();
    env::write(&"blobs have been read\n");

    // Step 2: Verify tx list
    let verifier = CelestiaVerifier::new(sov_celestia_adapter::verifier::RollupParams {
        namespace: ROLLUP_NAMESPACE,
    });

    let validity_condition = verifier
        .verify_relevant_tx_list::<NoOpHasher>(&header, &blobs, inclusion_proof, completeness_proof)
        .expect("Transaction list must be correct");
    env::write(&"Relevant txs verified\n");

    // Step 3: Apply blobs
    let mut app = create_zk_app_template::<Risc0Guest, CelestiaSpec>(prev_state_root_hash);

    let witness: ArrayWitness = guest.read_from_host();
    env::write(&"Witness have been read\n");

    env::write(&"Applying slot...\n");
    let result = app.apply_slot(witness, &header, &validity_condition, &mut blobs);

    env::write(&"Slot has been applied\n");

    // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/647
    let rewarded_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS).unwrap();
    let output = StateTransition::<CelestiaSpec, _> {
        initial_state_root: prev_state_root_hash,
        final_state_root: result.state_root.0,
        validity_condition,
        rewarded_address: rewarded_address.as_ref().to_vec(),
        slot_hash: header.hash(),
    };
    env::commit(&output);

    env::write(&"new state root committed\n");

    #[cfg(feature = "bench")]
    let end_cycles = env::get_cycle_count();

    #[cfg(feature = "bench")]
    {
        let tuple = (
            "Cycles per block".to_string(),
            (end_cycles - start_cycles) as u64,
        );
        let mut serialized = Vec::new();
        serialized.extend(tuple.0.as_bytes());
        serialized.push(0);
        let size_bytes = tuple.1.to_ne_bytes();
        serialized.extend(&size_bytes);

        // calculate the syscall name.
        let cycle_string = String::from("cycle_metrics\0");
        let metrics_syscall_name = unsafe {
            risc0_zkvm_platform::syscall::SyscallName::from_bytes_with_nul(cycle_string.as_ptr())
        };
        risc0_zkvm::guest::env::send_recv_slice::<u8, u8>(metrics_syscall_name, &serialized);
    }
}

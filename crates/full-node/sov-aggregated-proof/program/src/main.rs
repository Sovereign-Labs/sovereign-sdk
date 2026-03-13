#![no_main]

sp1_zkvm::entrypoint!(main);

use sha2::{Digest, Sha256};
use sov_aggregated_proof_shared::DeferredProofInput;

pub fn main() {
    let proof_inputs = sp1_zkvm::io::read::<Vec<DeferredProofInput>>();

    for (index, proof_input) in proof_inputs.iter().enumerate() {
        println!("[guest] verifying proof {index}");
        let public_values_digest: [u8; 32] = Sha256::digest(&proof_input.public_values).into();
        sp1_zkvm::lib::verify::verify_sp1_proof(&proof_input.vkey_hash, &public_values_digest);
        println!("[guest] verified proof {index}");
    }
}

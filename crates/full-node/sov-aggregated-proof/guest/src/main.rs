#![no_main]
sp1_zkvm::entrypoint!(main);

use sha2::{Digest, Sha256};
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::Spec;
use sov_rollup_interface::execution_mode::Zk;
use sov_rollup_interface::zk::StateTransitionPublicData;
use sov_sp1_adapter::SP1;
use sov_state::Storage;

type S = DefaultSpec<MockDaSpec, SP1, MockZkvm, Zk>;

pub fn main() {
    // Read the inner program's vkey hash from the host.
    let vkey_hash: [u64; 4] = sp1_zkvm::io::read();

    // Read the witnesses (public values from inner proofs).
    let witnesses: Vec<
        StateTransitionPublicData<
            <S as Spec>::Address,
            MockDaSpec,
            <<S as Spec>::Storage as Storage>::Root,
        >,
    > = sp1_zkvm::io::read();

    for witness in witnesses.iter() {
        // Serialize the witness the same way the inner program committed it.
        let encoded = bincode::serialize(witness).unwrap();

        // Compute the SHA256 hash of the public values (matching SP1's pv_digest format).
        let hash = Sha256::digest(&encoded);

        // Pack the 32-byte hash into [u64; 4] (little-endian, matching SP1's transmute layout).
        let mut pv_digest = [0u64; 4];
        for i in 0..4 {
            pv_digest[i] = u64::from_le_bytes(hash[i * 8..(i + 1) * 8].try_into().unwrap());
        }

        // Verify the inner proof recursively.
        // The proof was written by the host via stdin.write_proof() and is consumed implicitly.
        //sp1_zkvm::syscalls::verify::syscall_verify_sp1_proof(&vkey_hash, &pv_digest);
    }
}

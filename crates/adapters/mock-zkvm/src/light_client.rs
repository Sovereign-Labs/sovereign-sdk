//! Light-client support for rollups running the mock zkVM.

use sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash;
use sov_rollup_interface::zk::ZkLightClient;

use crate::{MockCodeCommitment, MockZkVerifier};

/// Light client for a rollup running the mock zkVM.
pub struct MockLightClient {
    /// The inner code commitment hash the rollup's state-transition proofs must use.
    expected_inner_vkey_hash: CodeCommitmentHash,
    /// The outer code commitment the node uses to produce aggregated proofs.
    outer_code_commitment: MockCodeCommitment,
}

impl MockLightClient {
    /// Creates a new [`MockLightClient`] that verifies proofs against
    /// `expected_inner_vkey_hash` and `outer_code_commitment`.
    pub fn new(
        expected_inner_vkey_hash: CodeCommitmentHash,
        outer_code_commitment: MockCodeCommitment,
    ) -> Self {
        Self {
            expected_inner_vkey_hash,
            outer_code_commitment,
        }
    }
}

impl ZkLightClient for MockLightClient {
    type Verifier = MockZkVerifier;

    fn outer_code_commitment(&self) -> &MockCodeCommitment {
        &self.outer_code_commitment
    }

    fn expected_inner_vkey_hash(&self) -> &CodeCommitmentHash {
        &self.expected_inner_vkey_hash
    }
}

//! Light-client support for rollups running the mock zkVM.

use anyhow::Context as _;
use serde::de::DeserializeOwned;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregateProofVerifier, AggregatedProofPublicData, CodeCommitmentHash,
    SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{CodeCommitmentTrait, ZkLightClient};

use crate::{MockCodeCommitment, MockZkVerifier};

/// Light client for a rollup running the mock zkVM.
pub struct MockLightClient {
    /// The inner code commitment hash the rollup's state-transition proofs must use.
    expected_inner_vkey_hash: CodeCommitmentHash,
    /// The outer code commitment the node uses to produce aggregated proofs.
    outer_code_commitment: MockCodeCommitment,
    /// Timeout-configured HTTP client used to fetch proofs from the node.
    http_client: reqwest::Client,
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
            http_client: Self::build_http_client(),
        }
    }
}

impl ZkLightClient for MockLightClient {
    fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    fn verify_aggregated_proof<Address, Da, Root>(
        &self,
        proof: SerializedAggregatedProof,
    ) -> anyhow::Result<AggregatedProofPublicData<Address, Da, Root>>
    where
        Address: DeserializeOwned,
        Da: DaSpec,
        Root: DeserializeOwned,
    {
        let expected_outer_vkey_hash = self.outer_code_commitment.to_hash();
        let public_data =
            AggregateProofVerifier::<MockZkVerifier>::new(self.outer_code_commitment.clone())
                .verify(&proof)
                .context("Aggregated proof verification failed")?;

        anyhow::ensure!(
            public_data.outer_vk_hash == expected_outer_vkey_hash,
            "Aggregated proof outer code commitment mismatch: proof claims {}, light client expects {}",
            public_data.outer_vk_hash,
            expected_outer_vkey_hash
        );

        anyhow::ensure!(
            public_data.inner_vkey_hash == self.expected_inner_vkey_hash,
            "Aggregated proof inner code commitment mismatch: proof claims {}, light client expects {}",
            public_data.inner_vkey_hash,
            self.expected_inner_vkey_hash
        );

        Ok(public_data)
    }
}

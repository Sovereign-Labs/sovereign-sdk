//! Off-circuit, host-side light-client support, shared across zkVMs.
//!
//! A light client verifies a rollup's latest aggregated ("outer") proof without
//! running a full node: given an already-fetched serialized proof (typically
//! retrieved from a running node's REST API), it cryptographically verifies the
//! proof and returns the committed public values. The verification is identical
//! for every zkVM, so it lives here; each adapter only declares its
//! [`ZkLightClient::Verifier`] and exposes its trusted outer code commitment and
//! inner verification-key hash.
//!
//! Fetching the proof over the network is intentionally left to the caller, so
//! this core crate stays free of an HTTP-client dependency. See the demo rollup's
//! `light-client` binary for an example of fetching and decoding the proof before
//! handing it to [`ZkLightClient::verify_aggregated_proof`].
//!
//! This module is only available under the `native` feature, since verifying a
//! proof is a host-side concern.

use serde::de::DeserializeOwned;

use crate::da::DaSpec;
use crate::zk::aggregated_proof::{
    AggregateProofVerifier, AggregatedProofPublicData, CodeCommitmentHash,
    SerializedAggregatedProof,
};
use crate::zk::{CodeCommitmentTrait, ZkVerifier};

/// Verifies a rollup's latest aggregated ("outer") proof without running a full
/// node.
///
/// A light client stores no state and replays no blocks. Given a serialized
/// aggregated proof, it derives the outer verification key for its zkVM,
/// cryptographically verifies the proof, and returns its public values.
///
/// Implementations are provided per zkVM by the respective adapter crates (e.g.
/// `sov_sp1_adapter::light_client::Sp1LightClient`,
/// `sov_mock_zkvm::light_client::MockLightClient`).
pub trait ZkLightClient {
    /// The zkVM verifier used to cryptographically verify aggregated proofs.
    type Verifier: ZkVerifier;

    /// The trusted outer code commitment the node uses to produce aggregated
    /// proofs.
    ///
    /// This is the full commitment (not just its hash) because verifying a proof
    /// requires constructing an [`AggregateProofVerifier`] from it; the expected
    /// outer hash is derived from it internally.
    fn outer_code_commitment(&self) -> &<Self::Verifier as ZkVerifier>::CodeCommitment;

    /// The inner code commitment hash the rollup's state-transition ("inner")
    /// proofs are expected to commit to.
    fn expected_inner_vkey_hash(&self) -> &CodeCommitmentHash;

    /// Verifies an already-fetched serialized aggregated proof against this
    /// client's trusted verification keys and returns its public values.
    ///
    /// Cryptographically verifies the proof against [`Self::outer_code_commitment`],
    /// then checks that the proof's committed outer and inner verification-key
    /// hashes match the trusted ones before returning the public values.
    fn verify_aggregated_proof<Address, Da, Root>(
        &self,
        proof: SerializedAggregatedProof,
    ) -> anyhow::Result<AggregatedProofPublicData<Address, Da, Root>>
    where
        Address: DeserializeOwned,
        Da: DaSpec,
        Root: DeserializeOwned,
    {
        let outer_code_commitment = self.outer_code_commitment();
        let expected_outer_vkey_hash = outer_code_commitment.to_hash();
        let public_data =
            AggregateProofVerifier::<Self::Verifier>::new(outer_code_commitment.clone())
                .verify(&proof)
                .map_err(|e| anyhow::anyhow!("Aggregated proof verification failed: {e:?}"))?;

        anyhow::ensure!(
            public_data.outer_vk_hash == expected_outer_vkey_hash,
            "Aggregated proof outer code commitment mismatch: proof claims {}, light client expects {}",
            public_data.outer_vk_hash,
            expected_outer_vkey_hash
        );

        let expected_inner_vkey_hash = self.expected_inner_vkey_hash();
        anyhow::ensure!(
            public_data.inner_vkey_hash == *expected_inner_vkey_hash,
            "Aggregated proof inner code commitment mismatch: proof claims {}, light client expects {}",
            public_data.inner_vkey_hash,
            expected_inner_vkey_hash
        );

        Ok(public_data)
    }
}

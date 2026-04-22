#![deny(missing_docs)]
//! # SP1 Adapter
//!
//! This crate contains an adapter allowing the SP1 to be used as a proof system for
//! Sovereign SDK rollups.
use std::fmt;
use std::fmt::Debug;

use crypto::{SP1PublicKey, SP1Signature};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash;
use sov_rollup_interface::zk::{CryptoSpec, ZkVerifier};

#[cfg(feature = "native")]
use crate::crypto::private_key::SP1PrivateKey;

pub mod crypto;
pub mod guest;
#[cfg(feature = "native")]
pub mod host;
#[cfg(feature = "native")]
pub mod network;

#[cfg(all(feature = "native", feature = "bench"))]
pub mod metrics;

/// Uniquely identifies a SP1 binary. Stored as a serialized version of `SP1VerifyingKey`.
///
/// Use the [`ZkvmHost::code_commitment`](sov_rollup_interface::zk::ZkvmHost) method to get the MethodId for a given binary.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SP1MethodId(pub [u32; 8]);

impl Debug for SP1MethodId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("SP1MethodId").field(&self.0).finish()
    }
}

impl sov_rollup_interface::zk::CodeCommitmentTrait for SP1MethodId {
    fn to_hash(&self) -> CodeCommitmentHash {
        CodeCommitmentHash::from_u32_array(self.0)
    }

    fn from_hash(hash: sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash) -> Self {
        Self(hash.to_u32_array())
    }
}

/// The cryptographic primitives provided by SP1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Copy, JsonSchema)]
pub struct SP1CryptoSpec;

impl CryptoSpec for SP1CryptoSpec {
    #[cfg(feature = "native")]
    type PrivateKey = SP1PrivateKey;
    type PublicKey = SP1PublicKey;
    type Hasher = sha2::Sha256;
    type Signature = SP1Signature;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        let admin_pubkey_bytes: [u8; 32] = [
            0xf1, 0xac, 0x96, 0xb6, 0xad, 0x3c, 0xd6, 0xbd, 0xda, 0xf2, 0xc2, 0x3f, 0x08, 0x9d,
            0xe7, 0x3a, 0x68, 0x16, 0xf8, 0x92, 0xc1, 0xaf, 0x34, 0x5d, 0xf7, 0x0f, 0x9a, 0x57,
            0x3a, 0x86, 0xba, 0xcb,
        ];

        // This will panic if the bytes are invalid, which is fine for a hardcoded constant
        let pub_key = ed25519_consensus::VerificationKey::try_from(admin_pubkey_bytes)
            .expect("Invalid admin public key bytes");

        SP1PublicKey { pub_key }
    }
}

/// A verifier for SP1 proofs.
#[derive(Default, Clone)]
pub struct SP1Verifier;

#[cfg(not(target_os = "zkvm"))]
impl ZkVerifier for SP1Verifier {
    type CodeCommitment = SP1MethodId;
    type CryptoSpec = SP1CryptoSpec;
    type Error = anyhow::Error;

    fn verify<T: DeserializeOwned>(
        serialized_proof: &[u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<T, Self::Error> {
        use core::borrow::Borrow;
        use slop_algebra::AbstractField;
        use slop_algebra::PrimeField32;

        let proof = decode_sp1_proof(serialized_proof)?;
        let is_mock = std::env::var("SP1_PROVER").ok().as_deref() == Some("mock");

        if !is_mock {
            let sp1_sdk::SP1Proof::Compressed(ref compressed) = proof.proof else {
                anyhow::bail!("SP1Verifier only supports compressed proofs");
            };

            let vkey_hash: [sp1_primitives::SP1Field; 8] = code_commitment
                .0
                .map(sp1_primitives::SP1Field::from_canonical_u32);

            sp1_verifier::compressed::SP1CompressedVerifier::new()
                .verify_compressed_with_public_values(
                    compressed.as_ref(),
                    proof.public_values.as_slice(),
                    &vkey_hash,
                )?;

            // `SP1CompressedVerifier` does not inspect `exit_code`; the SDK's
            // verifier does. Enforce SUCCESS here after the proof shape has been validated.
            let recursion_public_values: &sp1_recursion_executor::RecursionPublicValues<
                sp1_primitives::SP1Field,
            > = compressed.proof.public_values.as_slice().borrow();
            let exit_code = recursion_public_values.exit_code.as_canonical_u32();
            if exit_code != 0 {
                anyhow::bail!("SP1 proof has non-success exit code: {exit_code}");
            }
        }

        Ok(bincode::deserialize(proof.public_values.as_slice())?)
    }
}

/// The SP1 Zkvm.
#[derive(Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct SP1;

impl sov_rollup_interface::zk::Zkvm for SP1 {
    type Guest = crate::guest::SP1Guest;
    type Verifier = SP1Verifier;

    #[cfg(feature = "native")]
    type Host = crate::host::SP1Host;

    #[cfg(feature = "native")]
    type OuterHost = crate::host::SP1AggregationHost;

    #[cfg(feature = "native")]
    type Network = crate::network::SP1Network;
}

#[cfg(target_os = "zkvm")]
impl ZkVerifier for SP1Verifier {
    type CodeCommitment = SP1MethodId;

    type CryptoSpec = SP1CryptoSpec;

    type Error = anyhow::Error;

    fn verify<T: DeserializeOwned>(
        public_values: &[u8],
        vkey_hash: &Self::CodeCommitment,
    ) -> Result<T, Self::Error> {
        use sha2::Digest;
        let public_values_digest: [u8; 32] = sha2::Sha256::digest(public_values).into();
        sp1_zkvm::lib::verify::verify_sp1_proof(&vkey_hash.0, &public_values_digest);
        Ok(bincode::deserialize(public_values)?)
    }
}

/// Decodes a serialized SP1 proof.
#[cfg(not(target_os = "zkvm"))]
pub fn decode_sp1_proof(
    serialized_proof: &[u8],
) -> anyhow::Result<sp1_sdk::SP1ProofWithPublicValues> {
    Ok(bincode::deserialize(serialized_proof)?)
}

#[cfg(feature = "metrics")]
mod metrics {
    use std::io::Write;

    use sov_metrics::Metric;

    #[derive(Debug)]
    #[allow(dead_code)]
    pub(crate) enum ProverType {
        Cpu,
        Network,
    }

    impl ProverType {
        pub fn as_str(&self) -> &'static str {
            match self {
                ProverType::Cpu => "cpu",
                ProverType::Network => "network",
            }
        }
    }

    /// Metrics emitted when the SP1 proving network fulfills a proof request.
    #[derive(Debug)]
    pub(super) struct SP1ProofFulfillmentMetrics {
        /// The type of prover emitting the metric
        pub prover_type: ProverType,
        /// The hex-encoded proof request ID.
        pub request_id: String,
        /// Time from proof request creation to fulfillment, in seconds, as reported by the SP1
        /// network.
        pub fulfillment_duration_secs: u64,
    }

    impl Metric for SP1ProofFulfillmentMetrics {
        fn measurement_name(&self) -> &'static str {
            "sov_rollup_sp1_proof_fulfillment"
        }

        fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
            write!(
                buffer,
                "{},prover_type={} request_id=\"{}\",fulfillment_duration_secs={}i",
                self.measurement_name(),
                self.prover_type.as_str(),
                self.request_id,
                self.fulfillment_duration_secs,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_sovereign_admin_pubkey() {
        use sov_rollup_interface::crypto::PublicKey;
        use sov_rollup_interface::zk::CryptoSpec;

        use crate::SP1CryptoSpec;

        let pub_key = SP1CryptoSpec::sovereign_admin_pubkey();
        let credential_id = pub_key.credential_id();
        assert_eq!(
            credential_id.to_string(),
            "0xf1ac96b6ad3cd6bddaf2c23f089de73a6816f892c1af345df70f9a573a86bacb"
        );
    }
}

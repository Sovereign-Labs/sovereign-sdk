#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
mod notifier;
use borsh::{BorshDeserialize, BorshSerialize};
use thiserror::Error;
mod guest;
pub use guest::MockZkGuest;
#[cfg(feature = "native")]
mod host;
#[cfg(feature = "native")]
pub use host::MockZkvmHost;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
pub mod crypto;
use sov_rollup_interface::zk::{CryptoSpec, SerializedZkProof, Zkvm};

use crate::crypto::{Ed25519PublicKey, Ed25519Signature};

/// The cryptographic primitives provided for general purpose use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Copy, schemars::JsonSchema)]
pub struct MockZkvmCryptoSpec;

impl CryptoSpec for MockZkvmCryptoSpec {
    #[cfg(feature = "native")]
    type PrivateKey = crate::crypto::private_key::Ed25519PrivateKey;
    type PublicKey = Ed25519PublicKey;
    type Hasher = sha2::Sha256;
    type Signature = Ed25519Signature;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        let admin_pubkey_bytes: [u8; 32] = [
            0xf1, 0xac, 0x96, 0xb6, 0xad, 0x3c, 0xd6, 0xbd, 0xda, 0xf2, 0xc2, 0x3f, 0x08, 0x9d,
            0xe7, 0x3a, 0x68, 0x16, 0xf8, 0x92, 0xc1, 0xaf, 0x34, 0x5d, 0xf7, 0x0f, 0x9a, 0x57,
            0x3a, 0x86, 0xba, 0xcb,
        ];

        // This will panic if the bytes are invalid, which is fine for a hardcoded constant
        let pub_key = ed25519_dalek::VerifyingKey::from_bytes(&admin_pubkey_bytes)
            .expect("Invalid admin public key bytes");

        Ed25519PublicKey { pub_key }
    }
}

/// A mock zk virtual machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct MockZkvm;

impl Zkvm for MockZkvm {
    type Guest = MockZkGuest;
    type Verifier = MockZkVerifier;

    #[cfg(feature = "native")]
    type Host = crate::host::MockZkvmHost;

    #[cfg(feature = "native")]
    type OuterHost = crate::host::MockZkvmHost;
}
/// A mock commitment to a particular zkVM program.
#[derive(
    Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Default,
)]
pub struct MockCodeCommitment(pub [u8; 8]);

impl sov_rollup_interface::zk::CodeCommitmentTrait for MockCodeCommitment {
    fn to_hash(&self) -> sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash {
        // Pad the 8-byte mock commitment to 32 bytes to match the canonical hash layout.
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(&self.0);
        sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash(bytes)
    }

    fn from_hash(hash: sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash) -> Self {
        let mut bytes = [0u8; 8];
        let len = hash.0.len().min(8);
        bytes[..len].copy_from_slice(&hash.0[..len]);
        Self(bytes)
    }
}

/// An error that can occur when converting a byte vector to a `MockCodeCommitment`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MockCodeCommitmentError {
    /// The input was not 32 bytes long.
    #[error("MockCodeCommitment must be 32 bytes long, but the input was {found} bytes long")]
    InvalidLength {
        /// The size of the input.
        found: usize,
    },
}

/// A helper type capable of simulating invalid proofs.
#[derive(Serialize, Deserialize)]
struct MockProof {
    /// Is proof valid.
    is_valid: bool,
    /// Public input.
    pub_data: Vec<u8>,
    /// The commitment.
    code_commitment: MockCodeCommitment,
}

impl MockProof {
    /// Bincode-encodes this proof. Only the native host path produces
    /// `MockProof`s; verifiers read them via [`Self::deserialize`].
    #[cfg(feature = "native")]
    pub(crate) fn serialize(&self) -> bincode::Result<Vec<u8>> {
        bincode::serialize(self)
    }

    /// Bincode-decodes a proof from `bytes`.
    pub(crate) fn deserialize(bytes: &[u8]) -> bincode::Result<Self> {
        bincode::deserialize(bytes)
    }
}

/// The verifier for mock zk proofs.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct MockZkVerifier;

impl sov_rollup_interface::zk::ZkVerifier for MockZkVerifier {
    type CodeCommitment = MockCodeCommitment;

    type CryptoSpec = MockZkvmCryptoSpec;

    type Error = anyhow::Error;

    fn verify_with_pub_values<T: DeserializeOwned>(
        public_values: &sov_rollup_interface::zk::aggregated_proof::common::SerializedPubValues,
        code_commitment: &Self::CodeCommitment,
    ) -> Result<T, Self::Error> {
        // The mock encodes a complete `MockProof` in `pub_values.pub_values` —
        // mirroring SP1's deferred-proof channel, but in-band so the circuit
        // can run natively.
        let MockProof {
            is_valid,
            pub_data,
            code_commitment: claimed,
        } = MockProof::deserialize(&public_values.pub_values)?;
        if !is_valid {
            anyhow::bail!("Proof is not valid");
        }
        if &claimed != code_commitment {
            anyhow::bail!(
                "Code commitment mismatch: proof claims {claimed:?}, verifier expects {code_commitment:?}"
            );
        }
        Ok(bincode::deserialize(&pub_data)?)
    }

    fn verify_with_proof<T: DeserializeOwned>(
        serialized_proof: &SerializedZkProof,
        code_commitment: &Self::CodeCommitment,
    ) -> Result<T, Self::Error> {
        let MockProof {
            is_valid,
            pub_data: input,
            code_commitment: claimed,
        } = MockProof::deserialize(&serialized_proof.raw_proof)?;
        if !is_valid {
            anyhow::bail!("Proof is not valid");
        }
        if &claimed != code_commitment {
            anyhow::bail!(
                "Code commitment mismatch: proof claims {claimed:?}, verifier expects {code_commitment:?}"
            );
        }
        Ok(bincode::deserialize(&input)?)
    }

    fn extract_public_data<T: DeserializeOwned>(
        serialized_proof: &SerializedZkProof,
    ) -> Result<T, Self::Error> {
        let MockProof {
            pub_data: input, ..
        } = MockProof::deserialize(&serialized_proof.raw_proof)?;
        Ok(bincode::deserialize(&input)?)
    }
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::crypto::PublicKey;
    use sov_rollup_interface::zk::{ZkVerifier, ZkvmHost};

    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
    struct TestPublicData {
        hint: String,
    }

    #[test]
    fn test_sovereign_admin_pubkey() {
        let pub_key = MockZkvmCryptoSpec::sovereign_admin_pubkey();
        let credential_id = pub_key.credential_id();
        assert_eq!(
            credential_id.to_string(),
            "0xf1ac96b6ad3cd6bddaf2c23f089de73a6816f892c1af345df70f9a573a86bacb"
        );
    }

    #[test]
    fn test_mock_vm() -> anyhow::Result<()> {
        let mut vm = MockZkvmHost::new();
        vm.make_proof();
        // Inner mock proofs commit nothing (no guest to derive public output
        // from the hint); verification asserts validity + matching commitment.
        let proof = vm.add_hint_deferred_and_run(&(), Default::default())?;
        MockZkVerifier::verify_with_proof::<()>(&proof, &Default::default())?;
        Ok(())
    }

    #[test]
    fn test_proof_serialization() -> anyhow::Result<()> {
        let proof = MockZkvmHost::create_serialized_proof(true, "Valid");
        let verified_pub_data =
            MockZkVerifier::verify_with_proof::<TestPublicData>(&proof, &Default::default());

        assert!(verified_pub_data.is_ok());

        let proof = MockZkvmHost::create_serialized_proof(false, "Invalid");
        let verified_pub_data =
            MockZkVerifier::verify_with_proof::<TestPublicData>(&proof, &Default::default());

        assert!(verified_pub_data.is_err());

        Ok(())
    }

    #[test]
    fn test_verify_accepts_matching_code_commitment() -> anyhow::Result<()> {
        let commitment = MockCodeCommitment(*b"binary01");
        let mut vm = MockZkvmHost::new().with_code_commitment(commitment.clone());
        vm.make_proof();
        let proof = vm.add_hint_deferred_and_run(&(), Default::default())?;
        MockZkVerifier::verify_with_proof::<()>(&proof, &commitment)?;
        Ok(())
    }

    #[test]
    fn test_verify_rejects_mismatched_code_commitment() {
        let v1 = MockCodeCommitment(*b"binary01");
        let v2 = MockCodeCommitment(*b"binary02");

        let mut vm = MockZkvmHost::new().with_code_commitment(v1);
        vm.make_proof();
        let proof = vm
            .add_hint_deferred_and_run(&(), Default::default())
            .unwrap();

        let err = MockZkVerifier::verify_with_proof::<()>(&proof, &v2).unwrap_err();
        assert!(
            err.to_string().contains("Code commitment mismatch"),
            "expected commitment-mismatch error, got: {err}"
        );
    }
}

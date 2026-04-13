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
#[cfg(feature = "native")]
mod network;
#[cfg(feature = "native")]
pub use network::MockZkvmNetwork;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
pub mod crypto;
use sov_rollup_interface::zk::{CryptoSpec, Zkvm};

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
    type Network = crate::network::MockZkvmNetwork;
}
/// A mock commitment to a particular zkVM program.
#[derive(
    Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Default,
)]
pub struct MockCodeCommitment(pub [u8; 8]);

impl sov_rollup_interface::zk::CodeCommitmentTrait for MockCodeCommitment {
    fn to_hash(
        &self,
    ) -> anyhow::Result<sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash> {
        // Pad the 8-byte mock commitment to 32 bytes to match the canonical hash layout.
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(&self.0);
        Ok(sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash(bytes))
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
}

/// The verifier for mock zk proofs.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct MockZkVerifier;

impl sov_rollup_interface::zk::ZkVerifier for MockZkVerifier {
    type CodeCommitment = MockCodeCommitment;

    type CryptoSpec = MockZkvmCryptoSpec;

    type Error = anyhow::Error;

    fn verify<T: DeserializeOwned>(
        serialized_proof: &[u8],
        _code_commitment: &Self::CodeCommitment,
    ) -> Result<T, Self::Error> {
        let MockProof {
            is_valid,
            pub_data: input,
        } = bincode::deserialize(serialized_proof)?;
        if is_valid {
            Ok(bincode::deserialize(&input)?)
        } else {
            anyhow::bail!("Proof is not valid")
        }
    }
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::crypto::PublicKey;
    use sov_rollup_interface::zk::{ZkVerifier, ZkvmHost, ZkvmNetwork};

    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
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
        let pub_data = TestPublicData {
            hint: "Test".to_owned(),
        };

        let mut vm = MockZkvmHost::new();
        vm.add_hint(&pub_data);
        vm.make_proof();

        let proof = vm.run().unwrap();
        let verified_pub_data =
            MockZkVerifier::verify::<TestPublicData>(&proof, &Default::default())?;

        assert_eq!(verified_pub_data, pub_data);
        Ok(())
    }

    #[test]
    fn test_proof_serialization() -> anyhow::Result<()> {
        let proof = MockZkvmHost::create_serialized_proof(true, "Valid");
        let verified_pub_data =
            MockZkVerifier::verify::<TestPublicData>(&proof, &Default::default());

        assert!(verified_pub_data.is_ok());

        let proof = MockZkvmHost::create_serialized_proof(false, "Invalid");
        let verified_pub_data =
            MockZkVerifier::verify::<TestPublicData>(&proof, &Default::default());

        assert!(verified_pub_data.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_mock_network() -> anyhow::Result<()> {
        let network = MockZkvmNetwork::new(false);
        let pub_data = TestPublicData {
            hint: "NetworkTest".to_owned(),
        };

        let handle = network.add_hint_and_submit(&pub_data).await?;

        // Proof should be pending
        assert_eq!(network.poll(&handle).await?, None);

        // Complete the proof
        network.complete_proof(handle);

        // Now poll should return the proof bytes
        let proof_bytes = network
            .poll(&handle)
            .await?
            .expect("proof should be ready after complete_proof");

        let verified = MockZkVerifier::verify::<TestPublicData>(&proof_bytes, &Default::default())?;
        assert_eq!(verified, pub_data);
        Ok(())
    }

    #[tokio::test]
    async fn test_mock_network_auto_complete() -> anyhow::Result<()> {
        let network = MockZkvmNetwork::new(true);
        let pub_data = TestPublicData {
            hint: "AutoComplete".to_owned(),
        };

        let handle = network.add_hint_and_submit(&pub_data).await?;

        // Proof should be immediately ready
        let proof_bytes = network
            .poll(&handle)
            .await?
            .expect("auto-complete proof should be immediately ready");

        let verified = MockZkVerifier::verify::<TestPublicData>(&proof_bytes, &Default::default())?;
        assert_eq!(verified, pub_data);
        Ok(())
    }
}

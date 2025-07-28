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
use sov_rollup_interface::zk::{CryptoSpec, Proof, Zkvm};

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
}
/// A mock commitment to a particular zkVM program.
#[derive(
    Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Default,
)]
pub struct MockCodeCommitment(pub [u8; 8]);

impl sov_rollup_interface::zk::CodeCommitment for MockCodeCommitment {
    type DecodeError = MockCodeCommitmentError;

    fn encode(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    fn decode(value: &[u8]) -> Result<Self, Self::DecodeError> {
        if value.len() != 8 {
            return Err(MockCodeCommitmentError::InvalidLength { found: value.len() });
        }
        let mut contents = [0u8; 8];
        contents.copy_from_slice(value);
        Ok(Self(contents))
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

/// A type that is impossible to instantiate.
#[derive(Serialize, Deserialize)]
enum Empty {}

/// A helper type capable of simulating invalid proofs.
#[derive(Serialize, Deserialize)]
struct Inner {
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
        let proof: Proof<Empty, Inner> = bincode::deserialize(serialized_proof)?;
        match proof {
            Proof::PublicData(Inner {
                is_valid,
                pub_data: input,
            }) => {
                if is_valid {
                    Ok(bincode::deserialize(&input)?)
                } else {
                    anyhow::bail!("Proof is not valid")
                }
            }
            Proof::Full(_) => unimplemented!("MockZkVerifier doesn't support full zk proofs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::crypto::PublicKey;
    use sov_rollup_interface::zk::{CodeCommitment, ZkVerifier, ZkvmHost};

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

        let proof = vm.run(false).unwrap();
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

    #[test]
    fn mock_code_commitment_codec_roundtrip() {
        // Check a roundtrip with the "digest" type from risc0.
        // This ensures that our use of `from_ne_bytes` is correct on the target platform.
        let raw_data = [1; 8];
        let method_id = MockCodeCommitment(raw_data);
        let bytes = method_id.encode();
        let id = MockCodeCommitment::decode(&bytes).expect("Encoding is valid");
        assert_eq!(id.0, raw_data);

        // Assert that we return the expected error when the length is incorrect.
        let bytes = vec![1u8; 31];
        assert!(matches!(
            MockCodeCommitment::decode(&bytes),
            Err(MockCodeCommitmentError::InvalidLength { found: 31 })
        ));
    }
}

#![deny(missing_docs)]
//! # RISC0 Adapter
//!
//! This crate contains an adapter allowing the Risc0 to be used as a proof system for
//! Sovereign SDK rollups.
use crypto::{Risc0PublicKey, Risc0Signature};
use risc0_zkvm::sha::Digest;
#[cfg(not(target_os = "zkvm"))]
use risc0_zkvm::Journal;
#[cfg(not(target_os = "zkvm"))]
use risc0_zkvm::Receipt;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "zkvm"))]
use sov_rollup_interface::zk::Proof;
use sov_rollup_interface::zk::{CodeCommitment, CryptoSpec, ZkVerifier};
use thiserror::Error;

pub mod crypto;
pub mod guest;
#[cfg(feature = "native")]
pub mod host;

#[cfg(target_os = "zkvm")]
pub mod logging;
#[cfg(all(feature = "native", feature = "bench"))]
pub mod metrics;

/// Uniquely identifies a Risc0 binary. Roughly equivalent to
/// the hash of the ELF file.
///
/// Use the [`ZkvmHost::code_commitment`](sov_rollup_interface::zk::ZkvmHost) method to get the MethodId for a given binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Risc0MethodId([u32; 8]);

impl PartialEq<Digest> for Risc0MethodId {
    fn eq(&self, other: &Digest) -> bool {
        self.0 == other.as_words()
    }
}

impl PartialEq<[u32; 8]> for Risc0MethodId {
    fn eq(&self, other: &[u32; 8]) -> bool {
        &self.0 == other
    }
}

impl CodeCommitment for Risc0MethodId {
    type DecodeError = Risc0MethodIdError;

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32);
        for word in &self.0 {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    fn decode(data: &[u8]) -> Result<Self, Self::DecodeError> {
        if data.len() != 32 {
            return Err(Risc0MethodIdError::InvalidLength { found: data.len() });
        }
        let mut contents = [0u32; 8];
        for (idx, chunk) in data.chunks_exact(4).enumerate() {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(chunk);
            contents[idx] = u32::from_le_bytes(bytes);
        }
        Ok(Self(contents))
    }
}

/// An error that can occur when converting a byte vector to a `Risc0MethodId`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Risc0MethodIdError {
    /// The input was not 32 bytes long.
    #[error("Risc0MethodId must be 32 bytes long, but the input was {found} bytes long")]
    InvalidLength {
        /// The length of the input.
        found: usize,
    },
}

/// The cryptographic primitives provided by the Risc0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Copy, JsonSchema)]
pub struct Risc0CryptoSpec;

impl CryptoSpec for Risc0CryptoSpec {
    #[cfg(feature = "native")]
    type PrivateKey = crate::crypto::private_key::Risc0PrivateKey;
    type PublicKey = Risc0PublicKey;
    type Hasher = sha2::Sha256;
    type Signature = Risc0Signature;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        let admin_pubkey_bytes: [u8; 32] = [
            0xf1, 0xac, 0x96, 0xb6, 0xad, 0x3c, 0xd6, 0xbd, 0xda, 0xf2, 0xc2, 0x3f, 0x08, 0x9d,
            0xe7, 0x3a, 0x68, 0x16, 0xf8, 0x92, 0xc1, 0xaf, 0x34, 0x5d, 0xf7, 0x0f, 0x9a, 0x57,
            0x3a, 0x86, 0xba, 0xcb,
        ];

        // This will panic if the bytes are invalid, which is fine for a hardcoded constant
        let pub_key = ed25519_dalek::VerifyingKey::from_bytes(&admin_pubkey_bytes)
            .expect("Invalid admin public key bytes");

        Risc0PublicKey { pub_key }
    }
}

/// A verifier for Risc0 proofs.
#[derive(Default, Clone)]
pub struct Risc0Verifier;

#[cfg(not(target_os = "zkvm"))]
impl ZkVerifier for Risc0Verifier {
    type CodeCommitment = Risc0MethodId;
    type CryptoSpec = Risc0CryptoSpec;
    type Error = anyhow::Error;

    fn verify<T: DeserializeOwned>(
        serialized_proof: &[u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<T, Self::Error> {
        let proof: Proof<Receipt, Option<Journal>> = bincode::deserialize(serialized_proof)?;
        match proof {
            Proof::PublicData(_) => anyhow::bail!("Risc0Verifier supports only full proofs"),
            Proof::Full(receipt) => {
                receipt.verify(code_commitment.0)?;
                Ok(bincode::deserialize(&receipt.journal.bytes)?)
            }
        }
    }
}

/// The Risc0 Zkvm.
#[derive(Debug, Clone, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct Risc0;

impl sov_rollup_interface::zk::Zkvm for Risc0 {
    type Verifier = Risc0Verifier;
    type Guest = crate::guest::Risc0Guest;

    #[cfg(feature = "native")]
    type Host = crate::host::Risc0Host<'static>;
}

#[cfg(target_os = "zkvm")]
impl ZkVerifier for Risc0Verifier {
    type CodeCommitment = Risc0MethodId;

    type CryptoSpec = Risc0CryptoSpec;

    type Error = anyhow::Error;

    fn verify<T: DeserializeOwned>(
        _serialized_proof: &[u8],
        _code_commitment: &Self::CodeCommitment,
    ) -> Result<T, Self::Error> {
        // Implement this method once risc0 supports recursion: issue #633
        todo!("Implement once risc0 supports recursion: https://github.com/Sovereign-Labs/sovereign-sdk/issues/633")
    }
}

#[test]
fn test_sovereign_admin_pubkey() {
    use sov_rollup_interface::crypto::PublicKey;

    let pub_key = Risc0CryptoSpec::sovereign_admin_pubkey();
    let credential_id = pub_key.credential_id();
    assert_eq!(
        credential_id.to_string(),
        "0xf1ac96b6ad3cd6bddaf2c23f089de73a6816f892c1af345df70f9a573a86bacb"
    );
}

#[test]
fn risc0_method_id_codec_roundtrip() {
    // Check a roundtrip with the "digest" type from risc0.
    // This ensures that our use of `from_ne_bytes` is correct on the target platform.
    let raw_data = [1u32, 2, 3, 4, 5, 6, 7, 8];
    let method_id = Risc0MethodId(raw_data);
    let bytes = method_id.encode();
    let id = Risc0MethodId::decode(&bytes).expect("Encoding is valid");
    assert_eq!(id.0, raw_data);

    // Assert that we return the expected error when the length is incorrect.
    let bytes = vec![1u8; 31];
    assert!(matches!(
        Risc0MethodId::decode(&bytes),
        Err(Risc0MethodIdError::InvalidLength { found: 31 })
    ));
}

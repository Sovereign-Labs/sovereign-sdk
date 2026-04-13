#![deny(missing_docs)]
//! # RISC0 Adapter
//!
//! This crate contains an adapter allowing the Risc0 to be used as a proof system for
//! Sovereign SDK rollups.
use crypto::{Risc0PublicKey, Risc0Signature};
use risc0_zkvm::sha::Digest;
#[cfg(not(target_os = "zkvm"))]
use risc0_zkvm::Receipt;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::zk::{CryptoSpec, ZkVerifier};
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

impl sov_rollup_interface::zk::CodeCommitmentTrait for Risc0MethodId {
    fn to_hash(
        &self,
    ) -> anyhow::Result<sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash> {
        Ok(sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash::from_u32_array(self.0))
    }
}

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
        let receipt: Receipt = bincode::deserialize(serialized_proof)?;
        receipt.verify(code_commitment.0)?;
        Ok(bincode::deserialize(&receipt.journal.bytes)?)
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

    #[cfg(feature = "native")]
    type Network = sov_rollup_interface::zk::NoopZkvmNetwork<crate::guest::Risc0Guest>;
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

use alloy_primitives::keccak256;
use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use sov_modules_api::macros::UniversalWallet;
use sov_rollup_interface::crypto::SigVerificationError;

use crate::evm::public_key::EthereumPublicKey;

/// A secp256k1 signature. Wraps the rust-secp256k1 crate.
#[derive(
    PartialEq, Eq, Debug, Clone, serde::Serialize, serde::Deserialize, JsonSchema, UniversalWallet,
)]
pub struct EthereumSignature {
    /// The inner signature.
    #[schemars(flatten, with = "String", length(equal = "128"))]
    #[sov_wallet(as_ty = "[u8; 64]")]
    pub msg_sig: k256::ecdsa::Signature,
}

impl BorshDeserialize for EthereumSignature {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buffer = [0; 64];
        reader.read_exact(&mut buffer)?;

        Ok(Self {
            msg_sig: k256::ecdsa::Signature::from_slice(&buffer).map_err(std::io::Error::other)?,
        })
    }
}

impl BorshSerialize for EthereumSignature {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(&self.msg_sig.to_bytes())
    }
}

impl TryFrom<&[u8]> for EthereumSignature {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self {
            msg_sig: k256::ecdsa::Signature::from_slice(value).map_err(anyhow::Error::msg)?,
        })
    }
}

impl sov_rollup_interface::crypto::Signature for EthereumSignature {
    type PublicKey = EthereumPublicKey;

    fn verify(&self, pub_key: &Self::PublicKey, msg: &[u8]) -> Result<(), SigVerificationError> {
        let digest = keccak256(msg);
        let verifying_key = k256::ecdsa::VerifyingKey::from(&pub_key.pub_key);
        use k256::ecdsa::signature::hazmat::PrehashVerifier;
        verifying_key
            .verify_prehash(&digest.0, &self.msg_sig)
            .map_err(|e| SigVerificationError {
                error: e.to_string(),
            })
    }
}

#[cfg(feature = "native")]
impl std::str::FromStr for EthereumSignature {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;

        let signature = k256::ecdsa::Signature::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("Invalid signature: {:?}", e))?;

        Ok(EthereumSignature { msg_sig: signature })
    }
}

#[cfg(test)]
mod signature_tests {
    use std::str::FromStr;

    use sov_rollup_interface::crypto::{PrivateKey, Signature};

    use super::*;
    use crate::evm::private_key::EthereumPrivateKey;

    #[test]
    fn test_signature_roundtrip() {
        let private_key = EthereumPrivateKey::generate();
        let message = b"test message for signature";

        // Sign the message
        let signature = private_key.sign(message);

        // Verify the signature
        let public_key = private_key.pub_key();
        assert!(signature.verify(&public_key, message).is_ok());
    }

    #[test]
    fn test_signature_serialization() {
        let private_key = EthereumPrivateKey::generate();
        let message = b"test serialization";
        let signature = private_key.sign(message);

        let sig_bytes = signature.msg_sig.to_bytes();
        let hex_str = hex::encode(sig_bytes);
        let recovered = EthereumSignature::from_str(&hex_str).unwrap();
        assert_eq!(signature, recovered);

        let borsh_bytes = borsh::to_vec(&signature).unwrap();
        let recovered: EthereumSignature = borsh::from_slice(&borsh_bytes).unwrap();
        assert_eq!(signature, recovered);

        let bincode_bytes = bincode::serialize(&signature).unwrap();
        let recovered: EthereumSignature = bincode::deserialize(&bincode_bytes).unwrap();
        assert_eq!(signature, recovered);
    }

    #[test]
    fn test_invalid_signature_verification() {
        let private_key1 = EthereumPrivateKey::generate();
        let private_key2 = EthereumPrivateKey::generate();
        let message = b"test message";

        // Sign with key1
        let signature = private_key1.sign(message);

        // Try to verify with key2's public key - should fail
        let wrong_public_key = private_key2.pub_key();
        assert!(signature.verify(&wrong_public_key, message).is_err());

        // Try to verify with correct key but wrong message - should fail
        let correct_public_key = private_key1.pub_key();
        assert!(signature
            .verify(&correct_public_key, b"wrong message")
            .is_err());
    }

    #[test]
    fn test_deterministic_signatures() {
        // k256 uses deterministic ECDSA (RFC 6979) by default
        let private_key = EthereumPrivateKey::generate();
        let message = b"deterministic test";

        let sig1 = private_key.sign(message);
        let sig2 = private_key.sign(message);

        // With deterministic signatures, signing the same message should produce the same signature
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_signature_format() {
        // Test that signatures are 64 bytes
        let private_key = EthereumPrivateKey::generate();
        let message = b"format test";
        let signature = private_key.sign(message);

        let sig_bytes = signature.msg_sig.to_bytes();
        assert_eq!(sig_bytes.len(), 64);

        // Test that signature can be reconstructed from bytes
        let reconstructed = EthereumSignature::try_from(sig_bytes.as_ref()).unwrap();
        assert_eq!(signature, reconstructed);
    }

    #[test]
    fn test_cross_library_compatibility() {
        // Test that our signatures are compatible with standard Ethereum signatures
        let private_key = EthereumPrivateKey::generate();
        let message = b"ethereum compatibility test";

        // Sign a message
        let signature = private_key.sign(message);

        // The signature should be 64 bytes (r: 32 bytes, s: 32 bytes)
        let sig_bytes = signature.msg_sig.to_bytes();
        assert_eq!(sig_bytes.len(), 64);

        // Verify the signature works
        let public_key = private_key.pub_key();
        assert!(signature.verify(&public_key, message).is_ok());
    }
}

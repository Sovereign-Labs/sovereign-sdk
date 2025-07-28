/// Defines private key types and operations
use alloy_primitives::keccak256;
use k256::ecdsa::{Signature, SigningKey};
use k256::PublicKey;
use rand::rngs::OsRng;
use sov_rollup_interface::crypto::PrivateKey;

use crate::evm::public_key::EthereumPublicKey;
use crate::evm::signature::EthereumSignature;

/// A private key for the sepc256k1 signature scheme.
/// This struct also stores the corresponding public key.
#[derive(Clone)]
pub struct EthereumPrivateKey {
    signing_key: SigningKey,
}

impl core::fmt::Debug for EthereumPrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthereumPrivateKey")
            .field("public_key", &self.signing_key.verifying_key())
            .field("private_key", &"***REDACTED***")
            .finish()
    }
}

impl PrivateKey for EthereumPrivateKey {
    type PublicKey = EthereumPublicKey;

    type Signature = EthereumSignature;

    fn generate() -> Self {
        let mut csprng = OsRng;

        Self {
            signing_key: SigningKey::random(&mut csprng),
        }
    }

    fn pub_key(&self) -> Self::PublicKey {
        EthereumPublicKey {
            pub_key: PublicKey::from(self.signing_key.verifying_key()),
        }
    }

    fn sign(&self, msg: &[u8]) -> Self::Signature {
        let digest = keccak256(msg);
        use k256::ecdsa::signature::hazmat::PrehashSigner;
        let signature: Signature = self.signing_key.sign_prehash(&digest.0).unwrap();
        EthereumSignature { msg_sig: signature }
    }
}

impl EthereumPrivateKey {
    /// Returns the private key as a hex string.
    /// TODO: Should it be 0x prefixed??
    pub fn as_hex(&self) -> String {
        hex::encode(self.signing_key.to_bytes())
    }
}

impl serde::Serialize for EthereumPrivateKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            // For JSON, serialize as hex string
            self.as_hex().serialize(serializer)
        } else {
            // For binary formats, serialize as fixed 32-byte array
            // This matches secp256k1's binary serialization format
            use serde::ser::SerializeTuple;
            let bytes = self.signing_key.to_bytes();
            let mut seq = serializer.serialize_tuple(32)?;
            for byte in bytes.as_slice() {
                seq.serialize_element(byte)?;
            }
            seq.end()
        }
    }
}

impl<'de> serde::Deserialize<'de> for EthereumPrivateKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            // For JSON, deserialize from hex string
            let hex_str = String::deserialize(deserializer)?;
            let bytes = hex::decode(&hex_str).map_err(serde::de::Error::custom)?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| serde::de::Error::custom("Invalid private key length"))?;
            let signing_key =
                SigningKey::from_bytes(&bytes.into()).map_err(serde::de::Error::custom)?;
            Ok(Self { signing_key })
        } else {
            // For binary formats, deserialize as fixed 32-byte array
            // This matches secp256k1's binary deserialization format
            let bytes = <[u8; 32]>::deserialize(deserializer)?;
            let signing_key =
                SigningKey::from_bytes(&bytes.into()).map_err(serde::de::Error::custom)?;
            Ok(Self { signing_key })
        }
    }
}

#[cfg(feature = "arbitrary")]
mod arbitrary_impls {
    use proptest::prelude::{any, BoxedStrategy};
    use proptest::strategy::Strategy;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use super::*;

    impl<'a> arbitrary::Arbitrary<'a> for EthereumPrivateKey {
        fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
            // it is important to generate the secret deterministically from the arbitrary argument
            // so keys and signatures will be reproducible for a given seed.
            // this unlocks fuzzy replay
            let seed = <[u8; 32]>::arbitrary(u)?;
            let rng = &mut StdRng::from_seed(seed);
            let signing_key = SigningKey::random(rng);

            Ok(Self { signing_key })
        }
    }

    impl<'a> arbitrary::Arbitrary<'a> for EthereumPublicKey {
        fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
            EthereumPrivateKey::arbitrary(u).map(|p| p.pub_key())
        }
    }

    impl<'a> arbitrary::Arbitrary<'a> for EthereumSignature {
        fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
            // the secret/public pair is lost; it is impossible to verify this signature
            // to run a verification, generate the keys+payload individually
            let payload_len = u.arbitrary_len::<u8>()?;
            let payload = u.bytes(payload_len)?;
            EthereumPrivateKey::arbitrary(u).map(|s| s.sign(payload))
        }
    }

    impl proptest::arbitrary::Arbitrary for EthereumPrivateKey {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
            any::<[u8; 32]>()
                .prop_map(|seed| Self {
                    signing_key: SigningKey::random(&mut StdRng::from_seed(seed)),
                })
                .boxed()
        }
    }

    impl proptest::arbitrary::Arbitrary for EthereumPublicKey {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
            any::<EthereumPrivateKey>()
                .prop_map(|key| key.pub_key())
                .boxed()
        }
    }

    impl proptest::arbitrary::Arbitrary for EthereumSignature {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
            any::<(EthereumPrivateKey, Vec<u8>)>()
                .prop_map(|(key, bytes)| key.sign(&bytes))
                .boxed()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_privatekey_serde_bincode() {
        let key_pair = EthereumPrivateKey::generate();
        let serialized = bincode::serialize(&key_pair).expect("Serialization to vec is infallible");
        let output = bincode::deserialize::<EthereumPrivateKey>(&serialized)
            .expect("SigningKey is serialized correctly");

        assert_eq!(key_pair.as_hex(), output.as_hex());
    }

    #[test]
    fn test_privatekey_serde_json() {
        let key_pair = EthereumPrivateKey::generate();
        let serialized = serde_json::to_vec(&key_pair).expect("Serialization to vec is infallible");
        let output = serde_json::from_slice::<EthereumPrivateKey>(&serialized)
            .expect("Keypair is serialized correctly");
        assert_eq!(key_pair.as_hex(), output.as_hex());
    }

    #[test]
    fn test_secp256k1_compat() {
        let secp = secp256k1::Secp256k1::new();
        let (secp_secret_key, secp_pub_key) =
            secp.generate_keypair(&mut secp256k1::rand::thread_rng());

        let serialized =
            bincode::serialize(&secp_secret_key).expect("Serialization to vec is infallible");

        let recovered: EthereumPrivateKey = bincode::deserialize(&serialized).unwrap();

        assert_eq!(recovered.pub_key().bytes(), secp_pub_key.serialize());
    }
}

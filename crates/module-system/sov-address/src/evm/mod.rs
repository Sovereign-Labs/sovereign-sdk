mod address;
mod crypto_spec;
#[cfg(feature = "native")]
mod private_key;
mod public_key;
mod signature;

pub use address::{EthereumAddress, MultiAddressEvm};
pub use crypto_spec::EvmCryptoSpec;

#[cfg(all(test, feature = "arbitrary"))]
mod proptest_tests {
    use proptest::collection::vec;
    use proptest::prelude::*;
    use sov_modules_api::{PrivateKey, Signature};

    use super::*;
    use crate::evm::private_key::EthereumPrivateKey;
    use crate::evm::public_key::EthereumPublicKey;
    use crate::evm::signature::EthereumSignature;

    proptest! {
        #[test]
        fn pub_key_json_schema_is_valid(item in any::<EthereumPublicKey>()) {
            let serialized = serde_json::to_value(item).unwrap();
            let schema = serde_json::to_value(schemars::schema_for!(EthereumPublicKey)).unwrap();

            jsonschema::validate(&schema, &serialized).unwrap();
        }

        #[test]
        fn sig_json_schema_is_valid(item in any::<EthereumSignature>()) {
            let serialized = serde_json::to_value(item).unwrap();
            let schema = serde_json::to_value(schemars::schema_for!(EthereumSignature)).unwrap();

            jsonschema::validate(&schema, &serialized).unwrap();
        }

        #[test]
        fn sig_verification_works(msg in vec(any::<u8>(), 0..100)) {
            let key = EthereumPrivateKey::generate();
            let signature = key.sign(&msg);
            let pubkey = key.pub_key();
            assert!(signature.verify(&pubkey, &msg).is_ok());
        }

        #[test]
        fn borsh_roundtrip_correctness(
            addr in any::<EthereumAddress>(),
            pk in any::<EthereumPublicKey>(),
            sig in any::<EthereumSignature>()
        ) {
            let ser_addr = borsh::to_vec(&addr).unwrap();
            let de_addr: EthereumAddress = borsh::from_slice(&ser_addr).unwrap();
            prop_assert_eq!(addr, de_addr);

            let ser_pk = borsh::to_vec(&pk).unwrap();
            let de_pk: EthereumPublicKey = borsh::from_slice(&ser_pk).unwrap();
            prop_assert_eq!(pk, de_pk);

            let ser_sig = borsh::to_vec(&sig).unwrap();
            let de_sig: EthereumSignature = borsh::from_slice(&ser_sig).unwrap();
            prop_assert_eq!(sig, de_sig);
        }

        #[test]
        fn serde_roundtrip_correctness(
            addr in any::<EthereumAddress>(),
            pk in any::<EthereumPublicKey>(),
            sig in any::<EthereumSignature>()
        ) {
            let binary_addr = bincode::serialize(&addr).unwrap();
            let de_addr: EthereumAddress = bincode::deserialize(&binary_addr).unwrap();
            prop_assert_eq!(addr, de_addr);
            let json_addr = serde_json::to_string(&addr).unwrap();
            let de_addr: EthereumAddress = serde_json::from_str(&json_addr).unwrap();
            prop_assert_eq!(addr, de_addr);

            let pk_binary = bincode::serialize(&pk).unwrap();
            let de_pk: EthereumPublicKey = bincode::deserialize(&pk_binary).unwrap();
            assert_eq!(pk, de_pk);
            let pk_json = serde_json::to_string(&pk).unwrap();
            let de_pk: EthereumPublicKey = serde_json::from_str(&pk_json).unwrap();
            assert_eq!(pk, de_pk);

            let sig_binary = bincode::serialize(&sig).unwrap();
            let de_sig: EthereumSignature = bincode::deserialize(&sig_binary).unwrap();
            assert_eq!(sig, de_sig);
            let sig_json = serde_json::to_string(&sig).unwrap();
            let de_sig: EthereumSignature = serde_json::from_str(&sig_json).unwrap();
            assert_eq!(sig, de_sig);
        }

        #[test]
        fn borsh_deserialization_does_not_panic(bytes in prop::collection::vec(any::<u8>(), 0..128)) {
            // The result of the deserialization is not used.
            // The purpose of the test is to ensure that the call does not panic.
            let _ = <EthereumAddress as borsh::BorshDeserialize>::deserialize(&mut &bytes[..]);
            let _ = <EthereumPublicKey as borsh::BorshDeserialize>::deserialize(&mut &bytes[..]);
            let _ = <EthereumSignature as borsh::BorshDeserialize>::deserialize(&mut &bytes[..]);
        }

        #[test]
        fn try_from_slice_does_not_panic(bytes in prop::collection::vec(any::<u8>(), 0..128)) {
            let _ = EthereumAddress::try_from(bytes.as_slice());
            let _ = EthereumSignature::try_from(bytes.as_slice());
        }
    }
}

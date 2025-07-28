//! Defines useful cryptographic primitives that are needed by all sovereign-sdk rollups.
mod simple_hasher;
pub use simple_hasher::NoOpHasher;
mod signatures;
pub use signatures::*;
use sov_universal_wallet::UniversalWallet;

use crate as sov_rollup_interface; // Needed for UniversalWallet, as it requires global paths
use crate::common::HexHash;

/// Type that represents an identifier for an authorizer of the transaction.
/// The credential is a [`HexHash`].
/// For example, this can be a padded EVM address or a hash of a rollup public key.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    derive_more::Display,
    derive_more::FromStr,
    derive_more::From,
    UniversalWallet,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
pub struct CredentialId(pub HexHash);

impl schemars::JsonSchema for CredentialId {
    fn schema_name() -> String {
        "CredentialId".to_string()
    }

    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        HexHash::json_schema(gen)
    }
}

impl From<[u8; 32]> for CredentialId {
    fn from(value: [u8; 32]) -> Self {
        Self(HexHash::new(value))
    }
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use super::*;

    fn check_str_and_back(credential_id: CredentialId) {
        let s = credential_id.to_string();
        let credential_id_recovered = CredentialId::from_str(&s).unwrap();
        assert_eq!(credential_id, credential_id_recovered);
    }

    #[test]
    fn test_hash_str_and_back_simple() {
        for i in 0..1 {
            let credential_id = CredentialId(HexHash::new([i as u8; 32]));
            check_str_and_back(credential_id);
        }
    }

    #[test]
    fn test_removed_0x_prefix() {
        let credential_id = CredentialId(HexHash::new([10u8; 32]));
        let s = credential_id.to_string().replace("0x", "");
        let result = CredentialId::from_str(&s);
        assert!(result.is_err());
        assert_eq!("Missing 0x prefix", result.unwrap_err().to_string());
    }

    #[test_strategy::proptest]
    fn test_arbitrary_hash_str_and_back(credential_id: CredentialId) {
        check_str_and_back(credential_id);
    }

    #[test_strategy::proptest]
    fn json_schema_is_valid(item: CredentialId) {
        validate_schema(&item).unwrap();
    }

    #[allow(clippy::result_large_err)]
    fn validate_schema<T>(item: &T) -> Result<(), jsonschema::error::ValidationErrorKind>
    where
        T: schemars::JsonSchema + serde::Serialize,
    {
        let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        let json = serde_json::to_value(item).unwrap();

        jsonschema::validate(&schema, &json).map_err(|e| e.kind)
    }
}

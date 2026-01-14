use std::str::FromStr;

use alloy_primitives::{Address, U256};
use sov_state::{BcsCodec, EncodeLike};

/// The key to a policy, consisting of the payer and payee addresses with a separator.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, derive_more::Display)]
#[display(r#"{}/slot/{}"#, self.0, self.1)]
pub struct AccountStorageKey(pub Address, pub U256);

impl FromStr for AccountStorageKey {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some((addr, location)) = s.split_once("/slot/") else {
            anyhow::bail!("Invalid AccountStorageKey - missing '/slot'' separator");
        };

        Ok(AccountStorageKey(
            Address::from_str(addr)?,
            U256::from_str(location)?,
        ))
    }
}

impl EncodeLike<(&Address, &U256), AccountStorageKey> for BcsCodec {
    fn encode_like(&self, borrowed: &(&Address, &U256), writer: &mut impl std::io::Write) {
        self.encode_like(borrowed.0, writer);
        self.encode_like(borrowed.1, writer);
    }
}

#[test]
fn test_account_storage_key_encode_like() {
    use sov_state::StateItemEncoder;
    let key = AccountStorageKey(Address::from_slice(&[1; 20]), U256::from(0));
    // Use fully qualified path to disambiguate between local crate and dev-dependency
    let encoded_like =
        <BcsCodec as EncodeLike<(&Address, &U256), AccountStorageKey>>::encode_to_vec_like(
            &BcsCodec,
            &(&key.0, &key.1),
        );

    assert_eq!(&BcsCodec.encode_to_vec(&key), &encoded_like);
}

#[test]
fn test_account_storage_key_str_roundtrip() {
    let key = AccountStorageKey(Address::from_slice(&[1; 20]), U256::from(5));
    assert_eq!(
        key.to_string(),
        "0x0101010101010101010101010101010101010101/slot/5"
    );

    assert_eq!(AccountStorageKey::from_str(&key.to_string()).unwrap(), key);
}

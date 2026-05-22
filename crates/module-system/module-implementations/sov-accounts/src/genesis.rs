use anyhow::bail;
use schemars::JsonSchema;
use serde_with::{serde_as, DisplayFromStr};
use sov_modules_api::prelude::*;
use sov_modules_api::{CredentialId, CryptoSpec, GenesisState};

use crate::{AccountOwnerKey, Accounts};

/// Best-effort guard-rail: returns `true` if `address` cannot represent a valid
/// public key under `S::CryptoSpec`. A `false` result does **not** prove the
/// address is non-synthetic — see below for the limits.
///
/// For 32-byte address specs (e.g., Solana-style `Base58Address` with ed25519) this
/// rejects addresses whose bytes are on the curve — addresses that correspond to a
/// real keypair and therefore have a natural off-chain owner.
///
/// For address schemes shorter than the public-key size (default 28-byte `Address`,
/// 20-byte `EthereumAddress`), `TryFrom` errors on size and this check is a no-op:
/// those schemes lose information through truncation/hashing and cannot be checked
/// from the address bytes alone. Closing that gap requires a different address scheme.
fn is_synthetic_address<S: Spec>(address: &S::Address) -> bool {
    <<S::CryptoSpec as CryptoSpec>::PublicKey as TryFrom<Vec<u8>>>::try_from(
        address.as_ref().to_vec(),
    )
    .is_err()
}

/// Credential/address authorization data for genesis.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountData<Address> {
    /// Credential ID to authorize.
    #[serde_as(as = "DisplayFromStr")]
    pub credential_id: CredentialId,
    /// Address the credential may act as.
    pub address: Address,
}

/// Initial configuration for sov-accounts module.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(bound = "S: ::sov_modules_api::Spec", rename = "AccountConfig")]
pub struct AccountConfig<S: Spec> {
    /// Credential/address authorizations to initialize.
    pub accounts: Vec<AccountData<S::Address>>,
    /// Enable configured credential authorizations and `InsertCredentialId`.
    #[serde(default = "default_true")]
    pub enable_custom_account_mappings: bool,
}

fn default_true() -> bool {
    true
}

impl<S: Spec> Accounts<S> {
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as sov_modules_api::Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.enable_custom_account_mappings
            .set(&config.enable_custom_account_mappings, state)?;

        if !config.enable_custom_account_mappings {
            if !config.accounts.is_empty() {
                bail!(
                    "Custom account mapping is disabled, but accounts are provided: {:?}",
                    config.accounts
                )
            }
            return Ok(());
        }

        for acc in &config.accounts {
            let key = AccountOwnerKey::new(acc.address, acc.credential_id);
            if self.account_owners.get(&key, state)?.is_some() {
                bail!(
                    "Authorization already exists for address {} and credential {}",
                    acc.address,
                    acc.credential_id
                )
            }
            if !is_synthetic_address::<S>(&acc.address) {
                bail!(
                    "Genesis address {} is a valid public key on the configured curve. \
                     Only addresses with no natural keypair owner may be registered at genesis.",
                    acc.address,
                )
            }
            // TODO: We can write canonical address, needlessly...
            self.authorize_credential(&acc.address, &acc.credential_id, state)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_modules_api::capabilities::mocks::MockKernel;
    use sov_modules_api::{CredentialId, CryptoSpec, PublicKey, StateCheckpoint};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::{TestPublicKey, TestSpec};

    use super::*;

    fn fresh_state() -> StateCheckpoint<TestSpec> {
        let kernel = MockKernel::<TestSpec>::default();
        let storage = SimpleStorageManager::new().create_storage();
        StateCheckpoint::<TestSpec>::new(storage, &kernel)
    }

    fn account_data(
        credential_id_hex: &str,
        address_str: &str,
    ) -> AccountData<<TestSpec as Spec>::Address> {
        AccountData {
            credential_id: CredentialId::from_str(credential_id_hex).unwrap(),
            address: <TestSpec as Spec>::Address::from_str(address_str).unwrap(),
        }
    }

    #[test]
    fn test_config_serialization() {
        let pub_key = &TestPublicKey::from_str(
            "1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf",
        )
        .unwrap();

        let credential_id = pub_key.credential_id();

        let config = AccountConfig::<TestSpec> {
            accounts: vec![AccountData {
                credential_id,
                address: credential_id.into(),
            }],
            enable_custom_account_mappings: false,
        };

        let data = r#"
        {
            "accounts":[
                {
                    "credential_id":"0x1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf",
                    "address":"sov1rn2w9kw4jslx70gjtzwnrlhwdwmvz8nm3nvedgunvglzqp593hk"
                }
            ],
            "enable_custom_account_mappings":false
        }"#;

        let parsed_config: AccountConfig<TestSpec> = serde_json::from_str(data).unwrap();
        assert_eq!(parsed_config, config);
    }

    #[test]
    fn init_module_disabled_empty_accounts_succeeds() {
        let mut state = fresh_state();
        let config = AccountConfig::<TestSpec> {
            accounts: Vec::new(),
            enable_custom_account_mappings: false,
        };
        let mut gs = state.to_genesis_state_accessor::<Accounts<TestSpec>>(&config);
        let mut accounts = Accounts::<TestSpec>::default();

        accounts.init_module(&config, &mut gs).unwrap();

        assert_eq!(
            accounts
                .enable_custom_account_mappings
                .get(&mut gs)
                .unwrap(),
            Some(false),
        );
    }

    #[test]
    fn init_module_disabled_with_accounts_errors() {
        let mut state = fresh_state();
        let config = AccountConfig::<TestSpec> {
            accounts: vec![account_data(
                "0x1111111111111111111111111111111111111111111111111111111111111111",
                "sov1rn2w9kw4jslx70gjtzwnrlhwdwmvz8nm3nvedgunvglzqp593hk",
            )],
            enable_custom_account_mappings: false,
        };
        let mut gs = state.to_genesis_state_accessor::<Accounts<TestSpec>>(&config);
        let mut accounts = Accounts::<TestSpec>::default();

        let err = accounts.init_module(&config, &mut gs).unwrap_err();
        assert!(
            err.to_string()
                .contains("Custom account mapping is disabled"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn init_module_enabled_authorizes_each_account() {
        let mut state = fresh_state();
        // Two distinct addresses, plus a second credential authorized for the
        // first address — exercising the "multiple credentials per address"
        // path that distinguishes `account_owners` from a plain credential map.
        let entries = vec![
            account_data(
                "0x1111111111111111111111111111111111111111111111111111111111111111",
                "sov1rn2w9kw4jslx70gjtzwnrlhwdwmvz8nm3nvedgunvglzqp593hk",
            ),
            account_data(
                "0x2222222222222222222222222222222222222222222222222222222222222222",
                "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
            ),
            account_data(
                "0x3333333333333333333333333333333333333333333333333333333333333333",
                "sov1rn2w9kw4jslx70gjtzwnrlhwdwmvz8nm3nvedgunvglzqp593hk",
            ),
        ];
        let config = AccountConfig::<TestSpec> {
            accounts: entries.clone(),
            enable_custom_account_mappings: true,
        };
        let mut gs = state.to_genesis_state_accessor::<Accounts<TestSpec>>(&config);
        let mut accounts = Accounts::<TestSpec>::default();

        accounts.init_module(&config, &mut gs).unwrap();

        assert_eq!(
            accounts
                .enable_custom_account_mappings
                .get(&mut gs)
                .unwrap(),
            Some(true),
        );
        for acc in entries {
            let key = AccountOwnerKey::new(acc.address, acc.credential_id);
            assert_eq!(
                accounts.account_owners.get(&key, &mut gs).unwrap(),
                Some(true),
                "authorization missing for {}/{}",
                acc.address,
                acc.credential_id,
            );
        }
    }

    #[test]
    fn init_module_enabled_duplicate_account_errors() {
        let mut state = fresh_state();
        let duplicate = account_data(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "sov1rn2w9kw4jslx70gjtzwnrlhwdwmvz8nm3nvedgunvglzqp593hk",
        );
        let config = AccountConfig::<TestSpec> {
            accounts: vec![duplicate.clone(), duplicate],
            enable_custom_account_mappings: true,
        };
        let mut gs = state.to_genesis_state_accessor::<Accounts<TestSpec>>(&config);
        let mut accounts = Accounts::<TestSpec>::default();

        let err = accounts.init_module(&config, &mut gs).unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn is_synthetic_address_is_no_op_for_28_byte_addresses() {
        // For the default 28-byte `sov1...` Address with 32-byte ed25519 pubkeys,
        // TryFrom always errors on size, so every 28-byte address is treated as
        // synthetic. This is an honest no-op gate — the address scheme has lost
        // information through truncation and cannot be checked from bytes alone.
        let entry = account_data(
            "0x3333333333333333333333333333333333333333333333333333333333333333",
            "sov1rn2w9kw4jslx70gjtzwnrlhwdwmvz8nm3nvedgunvglzqp593hk",
        );
        assert_eq!(entry.address.as_ref().len(), 28);
        assert!(is_synthetic_address::<TestSpec>(&entry.address));
    }

    mod base58_spec {
        use sov_modules_api::configurable_spec::ConfigurableSpec;
        use sov_modules_api::execution_mode::Native;
        use sov_modules_api::{Base58Address, PrivateKey};
        use sov_test_utils::{MockDaSpec, MockZkvm, MockZkvmCryptoSpec, TestStorage};

        use super::*;

        // A 32-byte address spec (Solana-style) so the synthetic check has teeth.
        // For this spec, address.as_ref() is the same 32 bytes as an ed25519 pubkey.
        type SolanaLikeSpec = ConfigurableSpec<
            MockDaSpec,
            MockZkvm,
            MockZkvm,
            Base58Address,
            Native,
            MockZkvmCryptoSpec,
            TestStorage,
        >;

        type SolanaCryptoSpec = <SolanaLikeSpec as Spec>::CryptoSpec;
        type SolanaPrivateKey = <SolanaCryptoSpec as CryptoSpec>::PrivateKey;

        fn synthetic_base58_address() -> Base58Address {
            // SHA256 of an arbitrary label. ~50% of random 32-byte strings are on the
            // ed25519 curve, so if the first seed lands on-curve we walk a counter
            // until we find an off-curve one. In practice we exit on iteration 0 or 1.
            use sov_modules_api::digest::Digest;
            type Hasher = <SolanaCryptoSpec as CryptoSpec>::Hasher;
            for counter in 0u32..32 {
                let mut hasher = Hasher::new();
                hasher.update(b"sov_accounts::test::synthetic");
                hasher.update(counter.to_le_bytes());
                let bytes: [u8; 32] = hasher.finalize().into();
                let addr = Base58Address::from(bytes);
                if is_synthetic_address::<SolanaLikeSpec>(&addr) {
                    return addr;
                }
            }
            panic!("could not find an off-curve hash output in 32 tries");
        }

        #[test]
        fn is_synthetic_address_rejects_natural_pubkey() {
            // A real ed25519 keypair: its 32-byte pubkey IS the address bytes, and is
            // by construction on the curve. The predicate must say "not synthetic".
            let priv_key = SolanaPrivateKey::generate();
            let credential_id = priv_key.pub_key().credential_id();
            let natural_address = Base58Address::from(credential_id);
            assert!(!is_synthetic_address::<SolanaLikeSpec>(&natural_address));
        }

        #[test]
        fn is_synthetic_address_accepts_hash_derived() {
            let synth = synthetic_base58_address();
            assert!(is_synthetic_address::<SolanaLikeSpec>(&synth));
        }
    }
}

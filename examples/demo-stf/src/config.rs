use crate::app::generate_address;
#[cfg(feature = "native")]
use crate::runtime::GenesisConfig;
use election::ElectionConfig;
#[cfg(feature = "native")]
use serde::de::DeserializeOwned;
#[cfg(feature = "native")]
pub use sov_modules_api::default_context::DefaultContext;
#[cfg(feature = "native")]
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::Context;
use sov_modules_api::PublicKey;
pub use sov_state::config::Config as StorageConfig;
#[cfg(feature = "native")]
use std::fs::File;
#[cfg(feature = "native")]
use std::io::Read;
#[cfg(feature = "native")]
use std::path::Path;
use value_setter::ValueSetterConfig;

pub const TEST_SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
pub const LOCKED_AMOUNT: u64 = 200;
pub const TEST_SEQ_PUB_KEY_STR: &str = "seq_pub_key";
pub const TEST_TOKEN_NAME: &str = "sov-test-token";

#[cfg(feature = "native")]
pub fn from_toml_path<P: AsRef<Path>, R: DeserializeOwned>(path: P) -> anyhow::Result<R> {
    let mut contents = String::new();
    {
        let mut file = File::open(path)?;
        file.read_to_string(&mut contents)?;
    }

    let result: R = toml::from_str(&contents)?;

    Ok(result)
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub storage: StorageConfig,
}

#[cfg(feature = "native")]
///
/// * `value_setter_admin_private_key` - Private key for the ValueSetter module admin.
/// * `election_admin_private_key` - Private key for the Election module admin.
pub fn create_demo_config(
    initial_sequencer_balance: u64,
    value_setter_admin_private_key: &DefaultPrivateKey,
    election_admin_private_key: &DefaultPrivateKey,
) -> GenesisConfig<DefaultContext> {
    create_demo_genesis_config::<DefaultContext>(
        initial_sequencer_balance,
        generate_address::<DefaultContext>(TEST_SEQ_PUB_KEY_STR),
        TEST_SEQUENCER_DA_ADDRESS.to_vec(),
        value_setter_admin_private_key,
        election_admin_private_key,
    )
}

#[cfg(feature = "native")]
/// Creates config for a rollup with some default settings, the config is used in demos and tests.
pub fn create_demo_genesis_config<C: Context>(
    initial_sequencer_balance: u64,
    sequencer_address: C::Address,
    sequencer_da_address: Vec<u8>,
    value_setter_admin_private_key: &DefaultPrivateKey,
    election_admin_private_key: &DefaultPrivateKey,
) -> GenesisConfig<C> {
    let token_config: bank::TokenConfig<C> = bank::TokenConfig {
        token_name: TEST_TOKEN_NAME.to_owned(),
        address_and_balances: vec![(sequencer_address.clone(), initial_sequencer_balance)],
    };

    let bank_config = bank::BankConfig {
        tokens: vec![token_config],
    };

    let token_address = bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &bank::genesis::DEPLOYER,
        bank::genesis::SALT,
    );

    let sequencer_config = sequencer::SequencerConfig {
        seq_rollup_address: sequencer_address,
        seq_da_address: sequencer_da_address,
        coins_to_lock: bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    };

    let value_setter_config = ValueSetterConfig {
        admin: value_setter_admin_private_key.pub_key().to_address(),
    };

    let election_config = ElectionConfig {
        admin: election_admin_private_key.pub_key().to_address(),
    };

    GenesisConfig::new(
        sequencer_config,
        bank_config,
        election_config,
        value_setter_config,
        accounts::AccountConfig { pub_keys: vec![] },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::{tempdir, NamedTempFile};

    fn create_config_from(content: &str) -> NamedTempFile {
        let mut config_file = NamedTempFile::new().unwrap();
        config_file.write_all(content.as_bytes()).unwrap();
        config_file
    }

    #[test]
    fn test_correct_config() {
        let config = r#"
            [storage]
            path = "/tmp"
        "#;

        let config_file = create_config_from(config);

        let config: Config = from_toml_path(config_file.path()).unwrap();
        let expected = Config {
            storage: StorageConfig {
                path: PathBuf::from("/tmp"),
            },
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn test_incorrect_path() {
        // Not closed quote
        let config = r#"
            [storage]
            path = "/tmp
        "#;
        let config_file = create_config_from(config);

        let config: anyhow::Result<Config> = from_toml_path(config_file.path());

        assert!(config.is_err());
        let error = config.unwrap_err().to_string();
        let expected_error = format!(
            "{}{}{}",
            "TOML parse error at line 3, column 25\n  |\n3 |",
            "             path = \"/tmp\n  |                         ^\n",
            "invalid basic string\n"
        );
        assert_eq!(error, expected_error);
    }
    //
    #[test]
    fn test_non_existent_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("non_existing_config.toml");

        let config: anyhow::Result<Config> = from_toml_path(path);

        assert!(config.is_err());
        assert_eq!(
            config.unwrap_err().to_string(),
            "No such file or directory (os error 2)"
        );
    }
}

use sov_election::ElectionConfig;
use sov_evm::{AccountData, EvmConfig};
pub use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, Hasher, PublicKey, Spec};
pub use sov_state::config::Config as StorageConfig;
use sov_value_setter::ValueSetterConfig;

/// Creates config for a rollup with some default settings, the config is used in demos and tests.
use crate::runtime::GenesisConfig;

pub const DEMO_SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
pub const LOCKED_AMOUNT: u64 = 200;
pub const DEMO_SEQ_PUB_KEY_STR: &str = "seq_pub_key";
pub const DEMO_TOKEN_NAME: &str = "sov-demo-token";

pub fn create_demo_genesis_config<C: Context>(
    initial_sequencer_balance: u64,
    sequencer_address: C::Address,
    sequencer_da_address: Vec<u8>,
    value_setter_admin_private_key: &DefaultPrivateKey,
    election_admin_private_key: &DefaultPrivateKey,
) -> GenesisConfig<C> {
    let token_config: sov_bank::TokenConfig<C> = sov_bank::TokenConfig {
        token_name: DEMO_TOKEN_NAME.to_owned(),
        address_and_balances: vec![(sequencer_address.clone(), initial_sequencer_balance)],
    };

    let bank_config = sov_bank::BankConfig {
        tokens: vec![token_config],
    };

    let token_address = sov_bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &sov_bank::genesis::DEPLOYER,
        sov_bank::genesis::SALT,
    );

    let sequencer_registry_config = sov_sequencer_registry::SequencerConfig {
        seq_rollup_address: sequencer_address,
        seq_da_address: sequencer_da_address,
        coins_to_lock: sov_bank::Coins {
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

    let genesis_evm_address = hex::decode("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266")
        .unwrap()
        .try_into()
        .expect("EVM module initialized with invalid address");

    GenesisConfig::new(
        bank_config,
        sequencer_registry_config,
        election_config,
        value_setter_config,
        sov_accounts::AccountConfig { pub_keys: vec![] },
        EvmConfig {
            data: vec![AccountData {
                address: genesis_evm_address,
                balance: AccountData::balance(1000000000),
                code_hash: AccountData::empty_code(),
                code: vec![],
                nonce: 0,
            }],
        },
    )
}

pub fn generate_address<C: Context>(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    <C as Spec>::Address::from(hash)
}

pub fn create_demo_config(
    initial_sequencer_balance: u64,
    value_setter_admin_private_key: &DefaultPrivateKey,
    election_admin_private_key: &DefaultPrivateKey,
) -> GenesisConfig<DefaultContext> {
    create_demo_genesis_config::<DefaultContext>(
        initial_sequencer_balance,
        generate_address::<DefaultContext>(DEMO_SEQ_PUB_KEY_STR),
        DEMO_SEQUENCER_DA_ADDRESS.to_vec(),
        value_setter_admin_private_key,
        election_admin_private_key,
    )
}

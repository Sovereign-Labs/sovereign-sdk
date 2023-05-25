/// Creates config for a rollup with some default settings, the config is used in demos and tests.
use crate::runtime::GenesisConfig;
use election::ElectionConfig;
pub use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::Context;
use sov_modules_api::Hasher;
use sov_modules_api::PublicKey;
use sov_modules_api::Spec;
pub use sov_state::config::Config as StorageConfig;
use value_setter::ValueSetterConfig;

pub const TEST_SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
pub const LOCKED_AMOUNT: u64 = 200;
pub const TEST_SEQ_PUB_KEY_STR: &str = "seq_pub_key";
pub const TEST_TOKEN_NAME: &str = "sov-test-token";

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

pub fn generate_address<C: Context>(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    <C as Spec>::Address::from(hash)
}

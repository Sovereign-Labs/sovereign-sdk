use sov_chain_state::ChainStateConfig;
#[cfg(feature = "experimental")]
use sov_evm::{AccountData, EvmConfig, SpecId};
pub use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Context, PrivateKey, PublicKey};
use sov_rollup_interface::da::DaSpec;
pub use sov_state::config::Config as StorageConfig;
use sov_value_setter::ValueSetterConfig;

/// Creates config for a rollup with some default settings, the config is used in demos and tests.
use crate::runtime::GenesisConfig;

pub const DEMO_SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
pub const LOCKED_AMOUNT: u64 = 50;
pub const DEMO_SEQ_PUB_KEY_STR: &str = "seq_pub_key";
pub const DEMO_TOKEN_NAME: &str = "sov-demo-token";

pub fn create_demo_genesis_config<C: Context, Da: DaSpec>(
    initial_sequencer_balance: u64,
    sequencer_address: C::Address,
    sequencer_da_address: Vec<u8>,
    value_setter_admin_private_key: &DefaultPrivateKey,
    #[cfg(feature = "experimental")] evm_genesis_addresses: Vec<reth_primitives::Address>,
) -> GenesisConfig<C, Da> {
    let token_config: sov_bank::TokenConfig<C> = sov_bank::TokenConfig {
        token_name: DEMO_TOKEN_NAME.to_owned(),
        address_and_balances: vec![(sequencer_address.clone(), initial_sequencer_balance)],
        authorized_minters: vec![sequencer_address.clone()],
        salt: 0,
    };

    let bank_config = sov_bank::BankConfig {
        tokens: vec![token_config],
    };

    let token_address = sov_bank::get_genesis_token_address::<C>(
        &bank_config.tokens[0].token_name,
        bank_config.tokens[0].salt,
    );

    let sequencer_registry_config = sov_sequencer_registry::SequencerConfig {
        seq_rollup_address: sequencer_address,
        seq_da_address: sequencer_da_address,
        coins_to_lock: sov_bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
        is_preferred_sequencer: true,
    };

    let value_setter_config = ValueSetterConfig {
        admin: value_setter_admin_private_key.pub_key().to_address(),
    };

    let chain_state_config = ChainStateConfig {
        // TODO: Put actual value
        initial_slot_height: 0,
    };

    GenesisConfig::new(
        bank_config,
        sequencer_registry_config,
        (),
        chain_state_config,
        value_setter_config,
        sov_accounts::AccountConfig { pub_keys: vec![] },
        #[cfg(feature = "experimental")]
        get_evm_config(evm_genesis_addresses),
    )
}

// TODO: #840
#[cfg(feature = "experimental")]
fn get_evm_config(genesis_addresses: Vec<reth_primitives::Address>) -> EvmConfig {
    let data = genesis_addresses
        .into_iter()
        .map(|address| AccountData {
            address,
            balance: AccountData::balance(u64::MAX),
            code_hash: AccountData::empty_code(),
            code: vec![],
            nonce: 0,
        })
        .collect();

    EvmConfig {
        data,
        chain_id: 1,
        limit_contract_code_size: None,
        spec: vec![(0, SpecId::LATEST)].into_iter().collect(),
        ..Default::default()
    }
}

pub fn create_demo_config<Da: DaSpec>(
    initial_sequencer_balance: u64,
    value_setter_admin_private_key: &DefaultPrivateKey,
) -> GenesisConfig<DefaultContext, Da> {
    create_demo_genesis_config::<DefaultContext, Da>(
        initial_sequencer_balance,
        generate_address::<DefaultContext>(DEMO_SEQ_PUB_KEY_STR),
        DEMO_SEQUENCER_DA_ADDRESS.to_vec(),
        value_setter_admin_private_key,
        #[cfg(feature = "experimental")]
        Vec::default(),
    )
}

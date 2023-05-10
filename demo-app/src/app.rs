use crate::runtime::GenesisConfig;
use crate::runtime::Runtime;
use crate::tx_hooks_impl::DemoAppTxHooks;
use crate::tx_verifier_impl::DemoAppTxVerifier;
use sov_app_template::AppTemplate;
#[cfg(feature = "native")]
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_context::ZkDefaultContext;
#[cfg(feature = "native")]
use sov_modules_api::Address;
use sov_modules_api::Context;
//#[cfg(test)]
use sov_modules_api::{PublicKey, Spec};
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::Storage;
use sov_state::ZkStorage;
#[cfg(feature = "native")]
use sovereign_sdk::stf::ProverConfig;
use sovereign_sdk::stf::{StateTransitionRunner, ZkConfig};
#[cfg(test)]
use std::path::Path;

use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::Hasher;

//#[cfg(test)]
pub(crate) type C = DefaultContext;

pub struct DemoAppRunner<C: Context>(pub DemoApp<C>);

pub type ZkAppRunner = DemoAppRunner<ZkDefaultContext>;

#[cfg(feature = "native")]
pub type NativeAppRunner = DemoAppRunner<DefaultContext>;

pub type DemoApp<C> = AppTemplate<C, DemoAppTxVerifier<C>, Runtime<C>, DemoAppTxHooks<C>>;

pub use sov_app_template::Batch;

#[cfg(test)]
pub const SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
#[cfg(test)]
pub const LOCKED_AMOUNT: u64 = 200;
#[cfg(test)]
pub const SEQ_PUB_KEY_STR: &str = "seq_pub_key";
#[cfg(test)]
pub const TOKEN_NAME: &str = "Token0";

#[cfg(feature = "native")]
impl StateTransitionRunner<ProverConfig> for DemoAppRunner<DefaultContext> {
    type RuntimeConfig = &'static str;
    type Inner = DemoApp<DefaultContext>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let runtime = Runtime::new();
        let storage =
            ProverStorage::with_config(runtime_config).expect("Failed to open prover storage");
        let tx_verifier = DemoAppTxVerifier::new();
        let tx_hooks = DemoAppTxHooks::new();
        let app = AppTemplate::new(storage, runtime, tx_verifier, tx_hooks);
        Self(app)
    }

    fn inner(&self) -> &Self::Inner {
        &self.0
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        &mut self.0
    }
}

impl StateTransitionRunner<ZkConfig> for DemoAppRunner<ZkDefaultContext> {
    type RuntimeConfig = [u8; 32];
    type Inner = DemoApp<ZkDefaultContext>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let runtime = Runtime::new();
        let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
        let tx_verifier = DemoAppTxVerifier::new();
        let tx_hooks = DemoAppTxHooks::new();
        let app = AppTemplate::new(storage, runtime, tx_verifier, tx_hooks);
        Self(app)
    }

    fn inner(&self) -> &Self::Inner {
        &self.0
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        &mut self.0
    }
}

#[cfg(feature = "native")]
pub fn create_mock_context_genesis_config(
    sequencer_address: Address,
    sequencer_da_address: Vec<u8>,
) -> GenesisConfig<DefaultContext> {
    create_demo_genesis_config::<DefaultContext>(sequencer_address, sequencer_da_address)
}

pub fn create_demo_genesis_config<C: Context>(
    sequencer_address: C::Address,
    sequencer_da_address: Vec<u8>,
) -> GenesisConfig<C> {
    /*
    let token_config: bank::TokenConfig<C> = bank::TokenConfig {
        token_name: "sov-test-token".to_owned(),
        address_and_balances: vec![(sequencer_address.clone(), 10_000)],
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
            amount: 1000,
            token_address,
        },
    };
    GenesisConfig::new(
        sequencer_config,
        bank_config,
        (),
        (),
        accounts::AccountConfig { pub_keys: vec![] },
    )*/

    todo!()
}

#[cfg(test)]
pub(crate) fn create_sequencer_config(
    seq_rollup_address: <DefaultContext as Spec>::Address,
    token_address: <DefaultContext as Spec>::Address,
) -> sequencer::SequencerConfig<DefaultContext> {
    sequencer::SequencerConfig {
        seq_rollup_address,
        seq_da_address: SEQUENCER_DA_ADDRESS.to_vec(),
        coins_to_lock: bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    }
}

pub fn generate_address(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    Address::from(hash)
}

#[cfg(test)]
pub(crate) fn create_config(
    initial_sequencer_balance: u64,
) -> (
    GenesisConfig<DefaultContext>,
    DefaultPrivateKey,
    DefaultPrivateKey,
) {
    use value_setter::ValueSetterConfig;

    let seq_address = generate_address(SEQ_PUB_KEY_STR);

    let token_config: bank::TokenConfig<DefaultContext> = bank::TokenConfig {
        token_name: TOKEN_NAME.to_owned(),
        address_and_balances: vec![(seq_address.clone(), initial_sequencer_balance)],
    };

    let bank_config = bank::BankConfig {
        tokens: vec![token_config],
    };

    let token_address = bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &bank::genesis::DEPLOYER,
        bank::genesis::SALT,
    );

    let sequencer_config = create_sequencer_config(seq_address, token_address);

    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let value_setter_admin_pub_key = value_setter_admin_private_key.pub_key();
    let value_setter_config = ValueSetterConfig {
        admin: value_setter_admin_pub_key.to_address(),
    };

    let election_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_pub_key = election_admin_private_key.pub_key();
    let election_admin_address: <C as Spec>::Address = election_admin_pub_key.to_address();

    (
        GenesisConfig::new(
            sequencer_config,
            bank_config,
            election_admin_address,
            value_setter_config,
            accounts::AccountConfig { pub_keys: vec![] },
        ),
        value_setter_admin_private_key,
        election_admin_private_key,
    )
}

#[cfg(test)]
pub(crate) fn create_new_demo(path: impl AsRef<Path>) -> DemoApp<DefaultContext> {
    let runtime = Runtime::new();
    let storage = ProverStorage::with_path(path).unwrap();
    let tx_hooks = DemoAppTxHooks::new();
    let tx_verifier = DemoAppTxVerifier::new();
    AppTemplate::new(storage, runtime, tx_verifier, tx_hooks)
}

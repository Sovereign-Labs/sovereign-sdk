use crate::runtime::GenesisConfig;
use crate::runtime::Runtime;
use crate::tx_hooks_impl::DemoAppTxHooks;
use crate::tx_verifier_impl::DemoAppTxVerifier;
use sov_app_template::AppTemplate;
#[cfg(feature = "native")]
use sov_modules_api::mocks::MockContext;
use sov_modules_api::mocks::ZkMockContext;
#[cfg(feature = "native")]
use sov_modules_api::Address;
use sov_modules_api::Context;
#[cfg(test)]
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

#[cfg(test)]
pub(crate) type C = MockContext;

pub struct DemoAppRunner<C: Context>(pub DemoApp<C>);

pub type ZkAppRunner = DemoAppRunner<ZkMockContext>;

#[cfg(feature = "native")]
pub type NativeAppRunner = DemoAppRunner<MockContext>;

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
impl StateTransitionRunner<ProverConfig> for DemoAppRunner<MockContext> {
    type RuntimeConfig = &'static str;
    type Inner = DemoApp<MockContext>;

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

impl StateTransitionRunner<ZkConfig> for DemoAppRunner<ZkMockContext> {
    type RuntimeConfig = [u8; 32];
    type Inner = DemoApp<ZkMockContext>;

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
) -> GenesisConfig<MockContext> {
    create_demo_genesis_config::<MockContext>(sequencer_address, sequencer_da_address)
}

pub fn create_demo_genesis_config<C: Context>(
    sequencer_address: C::Address,
    sequencer_da_address: Vec<u8>,
) -> GenesisConfig<C> {
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
    )
}

#[cfg(test)]
pub(crate) fn create_sequencer_config(
    seq_rollup_address: <MockContext as Spec>::Address,
    token_address: <MockContext as Spec>::Address,
) -> sequencer::SequencerConfig<MockContext> {
    sequencer::SequencerConfig {
        seq_rollup_address,
        seq_da_address: SEQUENCER_DA_ADDRESS.to_vec(),
        coins_to_lock: bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    }
}

#[cfg(test)]
pub(crate) fn create_config(initial_sequencer_balance: u64) -> GenesisConfig<MockContext> {
    type C = MockContext;
    let pub_key = <C as Spec>::PublicKey::try_from(SEQ_PUB_KEY_STR).unwrap();
    let seq_address = pub_key.to_address::<<C as Spec>::Address>();

    let token_config: bank::TokenConfig<MockContext> = bank::TokenConfig {
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

    GenesisConfig::new(
        sequencer_config,
        bank_config,
        (),
        (),
        accounts::AccountConfig { pub_keys: vec![] },
    )
}

#[cfg(test)]
pub(crate) fn create_new_demo(path: impl AsRef<Path>) -> DemoApp<MockContext> {
    let runtime = Runtime::new();
    let storage = ProverStorage::with_path(path).unwrap();
    let tx_hooks = DemoAppTxHooks::new();
    let tx_verifier = DemoAppTxVerifier::new();
    AppTemplate::new(storage, runtime, tx_verifier, tx_hooks)
}

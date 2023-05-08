#[cfg(test)]
use crate::runtime::GenesisConfig;
use crate::runtime::Runtime;
use crate::tx_hooks_impl::DemoAppTxHooks;
use crate::tx_verifier_impl::DemoAppTxVerifier;
use sov_app_template::AppTemplate;
use sov_modules_api::mocks::{DefaultContext, ZkDefaultContext};
use sov_modules_api::Context;
#[cfg(test)]
use sov_modules_api::{PublicKey, Spec};
use sov_state::{ProverStorage, ZkStorage};
use sovereign_sdk::stf::{ProverConfig, StateTransitionRunner, ZkConfig};
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
pub(crate) type C = DefaultContext;

pub struct DemoAppRunner<C: Context>(pub DemoApp<C>);

pub type DemoApp<C> = AppTemplate<C, DemoAppTxVerifier<C>, Runtime<C>, DemoAppTxHooks<C>>;

#[cfg(test)]
pub const SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
#[cfg(test)]
pub const LOCKED_AMOUNT: u64 = 200;
#[cfg(test)]
pub const SEQ_PUB_KEY_STR: &str = "seq_pub_key";
#[cfg(test)]
pub const TOKEN_NAME: &str = "Token0";

impl StateTransitionRunner<ProverConfig> for DemoAppRunner<DefaultContext> {
    type RuntimeConfig = &'static str;
    type Inner = DemoApp<DefaultContext>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let runtime = Runtime::new();
        let storage =
            ProverStorage::with_path(runtime_config).expect("Failed to open prover storage");
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
        let storage = ZkStorage::new(runtime_config);
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

#[cfg(test)]
pub(crate) fn create_config(initial_sequencer_balance: u64) -> GenesisConfig<DefaultContext> {
    type C = DefaultContext;
    let pub_key = <C as Spec>::PublicKey::try_from(SEQ_PUB_KEY_STR).unwrap();
    let seq_address = pub_key.to_address::<<C as Spec>::Address>();

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

    GenesisConfig::new(
        sequencer_config,
        bank_config,
        (),
        (),
        accounts::AccountConfig { pub_keys: vec![] },
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

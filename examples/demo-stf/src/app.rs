#[cfg(feature = "native")]
use crate::config::Config;
#[cfg(feature = "native")]
use crate::runtime::GenesisConfig;
use crate::runtime::Runtime;
use crate::tx_hooks_impl::DemoAppTxHooks;
use crate::tx_verifier_impl::DemoAppTxVerifier;
use sov_app_template::AppTemplate;
pub use sov_app_template::Batch;
#[cfg(feature = "native")]
pub use sov_modules_api::default_context::DefaultContext;
pub use sov_modules_api::default_context::ZkDefaultContext;
#[cfg(feature = "native")]
pub use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, RpcRunner};
#[cfg(feature = "native")]
use sov_modules_api::PublicKey;
use sov_modules_api::{Hasher, Spec};
#[cfg(feature = "native")]
use sov_rollup_interface::stf::ProverConfig;
use sov_rollup_interface::stf::ZkConfig;
use sov_rollup_interface::stf::StateTransitionRunner;
use sov_rollup_interface::zk::traits::Zkvm;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::Storage;
use sov_state::ZkStorage;
use std::path::Path;

#[cfg(test)]
pub(crate) type C = DefaultContext;
pub struct DemoAppRunner<C: Context, Vm: Zkvm>(pub DemoApp<C, Vm>);
pub type ZkAppRunner<Vm> = DemoAppRunner<ZkDefaultContext, Vm>;

use bank::query::BankRpcImpl;
use election::query::ElectionRpcImpl;
use value_setter::query::ValueSetterRpcImpl;

use sov_modules_macros::expose_rpc;

use bank::query::BankRpcServer;
use election::query::ElectionRpcServer;
use value_setter::query::ValueSetterRpcServer;

#[cfg(feature = "native")]
pub type NativeAppRunner<Vm> = DemoAppRunner<DefaultContext, Vm>;

pub type DemoApp<C, Vm> = AppTemplate<C, DemoAppTxVerifier<C>, Runtime<C>, DemoAppTxHooks<C>, Vm>;

pub const SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
pub const LOCKED_AMOUNT: u64 = 200;
pub const SEQ_PUB_KEY_STR: &str = "seq_pub_key";
pub const TOKEN_NAME: &str = "sov-test-token";

#[cfg(feature = "native")]
#[expose_rpc((Bank<DefaultContext>,Election<DefaultContext>,ValueSetter<DefaultContext>))]
impl<Vm: Zkvm> StateTransitionRunner<ProverConfig, Vm> for DemoAppRunner<DefaultContext, Vm> {
    type RuntimeConfig = Config;
    type Inner = DemoApp<DefaultContext, Vm>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let runtime = Runtime::new();
        let storage = ProverStorage::with_config(runtime_config.storage)
            .expect("Failed to open prover storage");
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

impl<Vm: Zkvm> StateTransitionRunner<ZkConfig, Vm> for DemoAppRunner<ZkDefaultContext, Vm> {
    type RuntimeConfig = [u8; 32];
    type Inner = DemoApp<ZkDefaultContext, Vm>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let runtime = Runtime::new();
        let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
        let tx_verifier = DemoAppTxVerifier::new();
        let tx_hooks = DemoAppTxHooks::new();
        let app: AppTemplate<
            ZkDefaultContext,
            DemoAppTxVerifier<ZkDefaultContext>,
            Runtime<ZkDefaultContext>,
            DemoAppTxHooks<ZkDefaultContext>,
            Vm,
        > = AppTemplate::new(storage, runtime, tx_verifier, tx_hooks);
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
impl<Vm: Zkvm> RpcRunner for DemoAppRunner<DefaultContext, Vm> {
    type Context = DefaultContext;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage {
        self.inner().current_storage.clone()
    }
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
        generate_address::<DefaultContext>(SEQ_PUB_KEY_STR),
        SEQUENCER_DA_ADDRESS.to_vec(),
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
    use election::ElectionConfig;
    use value_setter::ValueSetterConfig;

    let token_config: bank::TokenConfig<C> = bank::TokenConfig {
        token_name: TOKEN_NAME.to_owned(),
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

#[cfg(feature = "native")]
pub fn create_new_demo(
    path: impl AsRef<Path>,
) -> DemoApp<DefaultContext, sov_rollup_interface::mocks::MockZkvm> {
    let runtime = Runtime::new();
    let storage = ProverStorage::with_path(path).unwrap();
    let tx_hooks = DemoAppTxHooks::new();
    let tx_verifier = DemoAppTxVerifier::new();
    AppTemplate::new(storage, runtime, tx_verifier, tx_hooks)
}

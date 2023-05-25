#[cfg(feature = "native")]
use crate::runner_config::Config;
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
use sov_modules_api::Context;
#[cfg(feature = "native")]
use sov_modules_api::RpcRunner;
use sov_modules_api::Spec;
#[cfg(feature = "native")]
use sov_rollup_interface::stf::ProverConfig;
use sov_rollup_interface::stf::StateTransitionRunner;
use sov_rollup_interface::stf::ZkConfig;
use sov_rollup_interface::zk::traits::Zkvm;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::Storage;
use sov_state::ZkStorage;

pub struct DemoAppRunner<C: Context, Vm: Zkvm>(pub DemoApp<C, Vm>);
pub type ZkAppRunner<Vm> = DemoAppRunner<ZkDefaultContext, Vm>;

#[cfg(feature = "native")]
use sov_bank::query::{BankRpcImpl, BankRpcServer};
#[cfg(feature = "native")]
use sov_election::query::{ElectionRpcImpl, ElectionRpcServer};
#[cfg(feature = "native")]
use sov_value_setter::query::{ValueSetterRpcImpl, ValueSetterRpcServer};

#[cfg(feature = "native")]
use sov_modules_macros::expose_rpc;

#[cfg(feature = "native")]
pub type NativeAppRunner<Vm> = DemoAppRunner<DefaultContext, Vm>;

pub type DemoApp<C, Vm> = AppTemplate<C, DemoAppTxVerifier<C>, Runtime<C>, DemoAppTxHooks<C>, Vm>;

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

#[cfg(feature = "native")]
use crate::runner_config::Config;
use crate::runtime::Runtime;
#[cfg(feature = "native")]
pub use sov_modules_api::default_context::DefaultContext;
pub use sov_modules_api::default_context::ZkDefaultContext;
#[cfg(feature = "native")]
pub use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::Context;
#[cfg(feature = "native")]
use sov_modules_api::RpcRunner;
#[cfg(feature = "native")]
use sov_modules_api::Spec;
use sov_modules_stf_template::AppTemplate;
pub use sov_modules_stf_template::Batch;
use sov_modules_stf_template::SequencerOutcome;
use sov_modules_stf_template::TxEffect;
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
#[cfg(feature = "native")]
use sov_rollup_interface::stf::ProverConfig;
use sov_rollup_interface::stf::ZkConfig;
use sov_rollup_interface::zk::traits::Zkvm;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::ZkStorage;

use sov_state::Storage;
use std::cell::RefCell;

pub struct DemoAppRunner<C: Context, Vm: Zkvm> {
    pub stf: DemoApp<C, Vm>,
    batch_builder: RefCell<FiFoStrictBatchBuilder<Runtime<C>, C>>,
}

pub type ZkAppRunner<Vm> = DemoAppRunner<ZkDefaultContext, Vm>;

#[cfg(feature = "native")]
use sov_bank::query::{BankRpcImpl, BankRpcServer};
#[cfg(feature = "native")]
use sov_election::query::{ElectionRpcImpl, ElectionRpcServer};
#[cfg(feature = "native")]
use sov_value_setter::query::{ValueSetterRpcImpl, ValueSetterRpcServer};

use crate::batch_builder::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
use sov_modules_macros::expose_rpc;
#[cfg(feature = "native")]
use sov_rollup_interface::services::batch_builder::BatchBuilder;

#[cfg(feature = "native")]
pub type NativeAppRunner<Vm> = DemoAppRunner<DefaultContext, Vm>;

pub type DemoApp<C, Vm> = AppTemplate<C, Runtime<C>, Vm>;

/// Batch receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
pub type DemoBatchReceipt = SequencerOutcome;
/// Tx receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
pub type DemoTxReceipt = TxEffect;

#[cfg(feature = "native")]
#[expose_rpc((Bank<DefaultContext>,Election<DefaultContext>,ValueSetter<DefaultContext>))]
impl<Vm: Zkvm> StateTransitionRunner<ProverConfig, Vm> for DemoAppRunner<DefaultContext, Vm> {
    type RuntimeConfig = Config;
    type Inner = DemoApp<DefaultContext, Vm>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let runtime = Runtime::new();
        let storage = ProverStorage::with_config(runtime_config.storage)
            .expect("Failed to open prover storage");
        let app = AppTemplate::new(storage, runtime);
        let batch_size_bytes = 1024 * 100; // 100 KB
        let batch_builder =
            FiFoStrictBatchBuilder::new(batch_size_bytes, u32::MAX as usize, Runtime::new());
        Self {
            stf: app,
            batch_builder: RefCell::new(batch_builder),
        }
    }

    fn inner(&self) -> &Self::Inner {
        &self.stf
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        &mut self.stf
    }
}

impl<Vm: Zkvm> StateTransitionRunner<ZkConfig, Vm> for DemoAppRunner<ZkDefaultContext, Vm> {
    type RuntimeConfig = [u8; 32];
    type Inner = DemoApp<ZkDefaultContext, Vm>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let runtime = Runtime::new();
        let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
        let app: AppTemplate<ZkDefaultContext, Runtime<ZkDefaultContext>, Vm> =
            AppTemplate::new(storage, runtime);

        let batch_size_bytes = 1024 * 100; // 100 KB
        let batch_builder =
            FiFoStrictBatchBuilder::new(batch_size_bytes, u32::MAX as usize, Runtime::new());
        Self {
            stf: app,
            batch_builder: RefCell::new(batch_builder),
        }
    }

    fn inner(&self) -> &Self::Inner {
        &self.stf
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        &mut self.stf
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
impl<Vm: Zkvm> BatchBuilder for DemoAppRunner<DefaultContext, Vm> {
    fn accept_tx(&self, tx: Vec<u8>) -> anyhow::Result<()> {
        self.batch_builder.borrow().accept_tx(tx)
    }

    fn get_next_blob(&self) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut batch_builder = self.batch_builder.borrow_mut();
        let working_set = sov_state::WorkingSet::new(self.inner().current_storage.clone());
        batch_builder.reset_working_set(working_set);
        batch_builder.get_next_blob()
    }
}

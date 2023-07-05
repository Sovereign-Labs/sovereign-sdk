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
pub use sov_modules_stf_template::Batch;
use sov_modules_stf_template::{AppTemplate, SequencerOutcome, TxEffect};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
#[cfg(feature = "native")]
use sov_rollup_interface::stf::ProverConfig;
use sov_rollup_interface::stf::ZkConfig;
use sov_rollup_interface::zk::traits::Zkvm;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::{Storage, ZkStorage};

use crate::batch_builder::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
use crate::runner_config::Config;
use crate::runtime::Runtime;

pub struct DemoAppRunner<C: Context, Vm: Zkvm> {
    pub stf: DemoApp<C, Vm>,
    pub batch_builder: Option<FiFoStrictBatchBuilder<Runtime<C>, C>>,
}

pub type ZkAppRunner<Vm> = DemoAppRunner<ZkDefaultContext, Vm>;

#[cfg(feature = "native")]
pub type NativeAppRunner<Vm> = DemoAppRunner<DefaultContext, Vm>;

pub type DemoApp<C, Vm> = AppTemplate<C, Runtime<C>, Vm>;

/// Batch receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
pub type DemoBatchReceipt = SequencerOutcome;
/// Tx receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
pub type DemoTxReceipt = TxEffect;

#[cfg(feature = "native")]
impl<Vm: Zkvm> StateTransitionRunner<ProverConfig, Vm> for DemoAppRunner<DefaultContext, Vm> {
    type RuntimeConfig = Config;
    type Inner = DemoApp<DefaultContext, Vm>;
    type BatchBuilder = FiFoStrictBatchBuilder<Runtime<DefaultContext>, DefaultContext>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let storage = ProverStorage::with_config(runtime_config.storage)
            .expect("Failed to open prover storage");
        let app = AppTemplate::new(storage.clone(), Runtime::default());
        let batch_size_bytes = 1024 * 100; // 100 KB
        let batch_builder = FiFoStrictBatchBuilder::new(
            batch_size_bytes,
            u32::MAX as usize,
            Runtime::default(),
            storage,
        );
        Self {
            stf: app,
            batch_builder: Some(batch_builder),
        }
    }

    fn inner(&self) -> &Self::Inner {
        &self.stf
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        &mut self.stf
    }

    fn take_batch_builder(&mut self) -> Option<Self::BatchBuilder> {
        self.batch_builder.take()
    }
}

impl<Vm: Zkvm> StateTransitionRunner<ZkConfig, Vm> for DemoAppRunner<ZkDefaultContext, Vm> {
    type RuntimeConfig = [u8; 32];
    type Inner = DemoApp<ZkDefaultContext, Vm>;
    type BatchBuilder = FiFoStrictBatchBuilder<Runtime<ZkDefaultContext>, ZkDefaultContext>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
        let app: AppTemplate<ZkDefaultContext, Runtime<ZkDefaultContext>, Vm> =
            AppTemplate::new(storage.clone(), Runtime::default());

        let batch_size_bytes = 1024 * 100; // 100 KB
        let batch_builder = FiFoStrictBatchBuilder::new(
            batch_size_bytes,
            u32::MAX as usize,
            Runtime::default(),
            storage,
        );
        Self {
            stf: app,
            batch_builder: Some(batch_builder),
        }
    }

    fn inner(&self) -> &Self::Inner {
        &self.stf
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        &mut self.stf
    }

    fn take_batch_builder(&mut self) -> Option<Self::BatchBuilder> {
        self.batch_builder.take()
    }
}

#[cfg(feature = "native")]
impl<Vm: Zkvm> RpcRunner for DemoAppRunner<DefaultContext, Vm> {
    type Context = DefaultContext;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage {
        self.inner().current_storage.clone()
    }
}

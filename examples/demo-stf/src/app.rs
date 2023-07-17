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
use sov_rollup_interface::da::BlobTransactionTrait;
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
#[cfg(feature = "native")]
use sov_rollup_interface::stf::ProverConfig;
use sov_rollup_interface::stf::ZkConfig;
use sov_rollup_interface::zk::Zkvm;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::{Storage, ZkStorage};

use crate::batch_builder::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
use crate::runner_config::Config;
use crate::runtime::Runtime;

pub struct DemoAppRunner<C: Context, Vm: Zkvm, B: BlobTransactionTrait> {
    pub stf: DemoApp<C, Vm, B>,
    pub batch_builder: Option<FiFoStrictBatchBuilder<Runtime<C>, C>>,
}

pub type ZkAppRunner<Vm, B> = DemoAppRunner<ZkDefaultContext, Vm, B>;

#[cfg(feature = "native")]
pub type NativeAppRunner<Vm, B> = DemoAppRunner<DefaultContext, Vm, B>;

pub type DemoApp<C, Vm, B> = AppTemplate<C, Runtime<C>, Vm, B>;

/// Batch receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
pub type DemoBatchReceipt = SequencerOutcome;
/// Tx receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
pub type DemoTxReceipt = TxEffect;

#[cfg(feature = "native")]
impl<Vm: Zkvm, B: BlobTransactionTrait> StateTransitionRunner<ProverConfig, Vm, B>
    for DemoAppRunner<DefaultContext, Vm, B>
{
    type RuntimeConfig = Config;
    type Inner = DemoApp<DefaultContext, Vm, B>;
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

impl<Vm: Zkvm, B: BlobTransactionTrait> StateTransitionRunner<ZkConfig, Vm, B>
    for DemoAppRunner<ZkDefaultContext, Vm, B>
{
    type RuntimeConfig = [u8; 32];
    type Inner = DemoApp<ZkDefaultContext, Vm, B>;
    type BatchBuilder = FiFoStrictBatchBuilder<Runtime<ZkDefaultContext>, ZkDefaultContext>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
        let app: AppTemplate<ZkDefaultContext, Runtime<ZkDefaultContext>, Vm, B> =
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
impl<Vm: Zkvm, B: BlobTransactionTrait> RpcRunner for DemoAppRunner<DefaultContext, Vm, B> {
    type Context = DefaultContext;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage {
        self.inner().current_storage.clone()
    }
}

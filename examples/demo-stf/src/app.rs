#[cfg(feature = "native")]
pub use sov_modules_api::default_context::DefaultContext;
pub use sov_modules_api::default_context::ZkDefaultContext;
#[cfg(feature = "native")]
pub use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
#[cfg(feature = "native")]
use sov_modules_api::Spec;
use sov_modules_stf_template::AppTemplate;
pub use sov_modules_stf_template::Batch;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::Zkvm;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::{Storage, ZkStorage};
#[cfg(feature = "native")]
use sov_stf_runner::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
use sov_stf_runner::StorageConfig;

use crate::runtime::Runtime;

#[cfg(feature = "native")]
pub struct App<Vm: Zkvm, Da: DaSpec> {
    pub stf: AppTemplate<DefaultContext, Da, Vm, Runtime<DefaultContext, Da>>,
    pub batch_builder: Option<FiFoStrictBatchBuilder<Runtime<DefaultContext, Da>, DefaultContext>>,
}

#[cfg(feature = "native")]
impl<Vm: Zkvm, Da: DaSpec> App<Vm, Da> {
    pub fn new(storage_config: StorageConfig) -> Self {
        let storage =
            ProverStorage::with_config(storage_config).expect("Failed to open prover storage");
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

    pub fn get_storage(&self) -> <DefaultContext as Spec>::Storage {
        self.stf.current_storage.clone()
    }
}

pub fn create_zk_app_template<Vm: Zkvm, Da: DaSpec>(
    runtime_config: [u8; 32],
) -> AppTemplate<ZkDefaultContext, Da, Vm, Runtime<ZkDefaultContext, Da>> {
    let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
    AppTemplate::new(storage, Runtime::default())
}

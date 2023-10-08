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
use sov_sequencer::batch_builder::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
use sov_state::config::Config as StorageConfig;
use sov_state::ZkStorage;
#[cfg(feature = "native")]
use sov_state::{ProverStorage, Storage};

use crate::runtime::Runtime;

/// Contains StateTransitionFunction and other necessary dependencies needed for implementing a full node.
#[cfg(feature = "native")]
pub struct App<Vm: Zkvm, Da: DaSpec> {
    /// Concrete state transition function.
    pub stf: AppTemplate<DefaultContext, Da, Vm, Runtime<DefaultContext, Da>>,
    /// Batch builder.
    pub batch_builder: Option<FiFoStrictBatchBuilder<Runtime<DefaultContext, Da>, DefaultContext>>,
}

#[cfg(feature = "native")]
impl<Vm: Zkvm, Da: DaSpec> App<Vm, Da> {
    /// Creates a new `App`.
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

/// Contains StateTransitionFunction for the `zk` context.
pub struct ZkApp<Vm: Zkvm, Da: DaSpec> {
    pub stf: AppTemplate<ZkDefaultContext, Da, Vm, Runtime<ZkDefaultContext, Da>>,
}

impl<Vm: Zkvm, Da: DaSpec> Default for ZkApp<Vm, Da> {
    fn default() -> Self {
        let storage = ZkStorage::new();
        Self {
            stf: AppTemplate::new(storage, Runtime::default()),
        }
    }
}

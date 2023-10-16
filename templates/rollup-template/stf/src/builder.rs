//! This module implements the batch builder for the rollup.
//! To swap out the batch builder, simply replace the
//! FiFoStrictBatchBuilder in `StfWithBuilder` with a type of your choosing.
use sov_modules_api::default_context::DefaultContext;
#[cfg(feature = "native")]
use sov_modules_api::Spec;
use sov_modules_api::{DaSpec, Zkvm};
use sov_modules_stf_template::AppTemplate;
#[cfg(feature = "native")]
use sov_sequencer::batch_builder::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
use sov_state::{ProverStorage, Storage};

use super::runtime::Runtime;

/// The "native" version of the STF and a batch builder
pub struct StfWithBuilder<Vm: Zkvm, Da: DaSpec> {
    pub stf: AppTemplate<DefaultContext, Da, Vm, Runtime<DefaultContext, Da>>,
    pub batch_builder: Option<FiFoStrictBatchBuilder<Runtime<DefaultContext, Da>, DefaultContext>>,
}

#[cfg(feature = "native")]
impl<Vm: Zkvm, Da: DaSpec> StfWithBuilder<Vm, Da> {
    /// Create a new rollup instance
    pub fn new(storage_config: sov_stf_runner::StorageConfig) -> Self {
        let config = sov_state::config::Config {
            path: storage_config.path,
        };

        let storage = ProverStorage::with_config(config).expect("Failed to open prover storage");
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

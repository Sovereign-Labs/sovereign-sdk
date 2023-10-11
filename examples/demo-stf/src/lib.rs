#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
pub mod cli;
#[cfg(feature = "native")]
pub mod genesis_config;
mod hooks_impl;
pub mod runtime;
#[cfg(test)]
mod tests;
use runtime::Runtime;
#[cfg(feature = "native")]
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_context::ZkDefaultContext;
#[cfg(feature = "native")]
use sov_modules_api::Spec;
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::da::{DaSpec, DaVerifier};
use sov_rollup_interface::zk::Zkvm;
#[cfg(feature = "native")]
use sov_sequencer::batch_builder::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
use sov_state::config::Config as StorageConfig;
use sov_state::ZkStorage;
#[cfg(feature = "native")]
use sov_state::{ProverStorage, Storage};
use sov_stf_runner::verifier::StateTransitionVerifier;

/// A verifier for the demo rollup
pub type AppVerifier<DA, Zk> = StateTransitionVerifier<
    AppTemplate<
        ZkDefaultContext,
        <DA as DaVerifier>::Spec,
        Zk,
        Runtime<ZkDefaultContext, <DA as DaVerifier>::Spec>,
    >,
    DA,
    Zk,
>;

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

    /// Gets underlying storage.
    pub fn get_storage(&self) -> <DefaultContext as Spec>::Storage {
        self.stf.current_storage.clone()
    }
}

/// Create `StateTransitionFunction` for Zk context.
pub fn create_zk_app_template<Vm: Zkvm, Da: DaSpec>(
) -> AppTemplate<ZkDefaultContext, Da, Vm, Runtime<ZkDefaultContext, Da>> {
    let storage = ZkStorage::new();
    AppTemplate::new(storage, Runtime::default())
}

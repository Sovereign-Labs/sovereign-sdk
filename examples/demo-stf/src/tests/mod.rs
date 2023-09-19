use std::path::Path;

use sov_modules_api::{default_context::DefaultContext, DaSpec};
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::mocks::MockDaSpec;
use sov_state::ProverStorage;

use crate::{
    genesis_config::{get_genesis_config, DemoConfiguration, DEMO_SEQUENCER_DA_ADDRESS},
    runtime::Runtime,
};

mod da_simulation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;
pub(crate) type Da = MockDaSpec;

pub(crate) fn create_new_demo(
    path: impl AsRef<Path>,
) -> AppTemplate<
    DefaultContext,
    Da,
    sov_rollup_interface::mocks::MockZkvm,
    Runtime<DefaultContext, Da>,
> {
    let runtime = Runtime::default();
    let storage = ProverStorage::with_path(path).unwrap();
    AppTemplate::new(storage, runtime)
}

pub(crate) fn create_demo_config<Da: DaSpec>() -> DemoConfiguration<DefaultContext, Da> {
    get_genesis_config::<DefaultContext, Da>(
        DEMO_SEQUENCER_DA_ADDRESS.to_vec(),
        #[cfg(feature = "experimental")]
        Vec::default(),
    )
}

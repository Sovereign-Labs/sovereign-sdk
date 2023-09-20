use std::path::Path;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::DaSpec;
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::mocks::MockDaSpec;
use sov_state::ProverStorage;

use crate::genesis_config::get_genesis_config;
use crate::runtime::{GenesisConfig, Runtime};

mod da_simulation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;
pub(crate) type Da = MockDaSpec;

pub(crate) const TEST_SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];

pub(crate) fn create_new_app_template_for_tests(
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

pub(crate) fn get_genesis_config_for_tests<Da: DaSpec>() -> GenesisConfig<DefaultContext, Da> {
    get_genesis_config::<DefaultContext, Da>(
        Da::Address::try_from(&TEST_SEQUENCER_DA_ADDRESS).unwrap(),
        #[cfg(feature = "experimental")]
        Vec::default(),
    )
}

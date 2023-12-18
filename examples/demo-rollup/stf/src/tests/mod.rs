use std::path::Path;

use sov_mock_da::MockDaSpec;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::DaSpec;
use sov_modules_stf_blueprint::kernels::basic::BasicKernel;
use sov_modules_stf_blueprint::StfBlueprint;
use sov_prover_storage_manager::ProverStorageManager;
use sov_state::DefaultStorageSpec;

use crate::genesis_config::{get_genesis_config, GenesisPaths};
use crate::runtime::{GenesisConfig, Runtime};

mod da_simulation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;
pub(crate) type Da = MockDaSpec;

pub(crate) type RuntimeTest = Runtime<DefaultContext, Da>;
pub(crate) type StfBlueprintTest =
    StfBlueprint<DefaultContext, Da, sov_mock_zkvm::MockZkvm, RuntimeTest, BasicKernel<C>>;

pub(crate) fn create_storage_manager_for_tests(
    path: impl AsRef<Path>,
) -> ProverStorageManager<MockDaSpec, DefaultStorageSpec> {
    let config = sov_state::config::Config {
        path: path.as_ref().to_path_buf(),
    };
    ProverStorageManager::new(config).unwrap()
}

pub(crate) fn get_genesis_config_for_tests<Da: DaSpec>() -> GenesisConfig<DefaultContext, Da> {
    get_genesis_config::<DefaultContext, Da>(&GenesisPaths::from_dir(
        "../../test-data/genesis/integration-tests",
    ))
    .unwrap()
}

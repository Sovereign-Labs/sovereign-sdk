use std::path::Path;

use sov_mock_da::MockDaSpec;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::DaSpec;
use sov_modules_stf_blueprint::kernels::basic::{BasicKernel, BasicKernelGenesisConfig};
use sov_modules_stf_blueprint::{GenesisParams, StfBlueprint};
use sov_prover_storage_manager::ProverStorageManager;
use sov_state::DefaultStorageSpec;
use sov_stf_runner::read_json_file;

use crate::genesis_config::{get_genesis_config, GenesisPaths};
use crate::runtime::{GenesisConfig, Runtime};

mod da_simulation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;
pub(crate) type Da = MockDaSpec;

pub(crate) type RuntimeTest = Runtime<DefaultContext, Da>;
pub(crate) type StfBlueprintTest = StfBlueprint<
    DefaultContext,
    Da,
    sov_mock_zkvm::MockZkvm<<Da as DaSpec>::ValidityCondition>,
    RuntimeTest,
    BasicKernel<C, Da>,
>;

pub(crate) fn create_storage_manager_for_tests(
    path: impl AsRef<Path>,
) -> ProverStorageManager<MockDaSpec, DefaultStorageSpec> {
    let config = sov_state::config::Config {
        path: path.as_ref().to_path_buf(),
    };
    ProverStorageManager::new(config).unwrap()
}

pub(crate) fn get_genesis_config_for_tests<Da: DaSpec>(
) -> GenesisParams<GenesisConfig<DefaultContext, Da>, BasicKernelGenesisConfig<DefaultContext, Da>>
{
    let integ_test_conf_dir: &Path = "../../test-data/genesis/integration-tests".as_ref();
    let rt_params =
        get_genesis_config::<DefaultContext, Da>(&GenesisPaths::from_dir(integ_test_conf_dir))
            .unwrap();

    let chain_state = read_json_file(integ_test_conf_dir.join("chain_state.json")).unwrap();
    let kernel_params = BasicKernelGenesisConfig { chain_state };
    GenesisParams {
        runtime: rt_params,
        kernel: kernel_params,
    }
}

use demo_stf::MultiAddressEvmSolana;
use sov_celestia_adapter::verifier::CelestiaSpec;
use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::execution_mode::Native;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;

use crate::SoakHasher;

type CelestiaNativeStorage =
    NomtProverStorage<DefaultStorageSpec<SoakHasher>, <CelestiaSpec as DaSpec>::SlotHash>;
pub type CelestiaRollupSpec = ConfigurableSpec<
    CelestiaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
    MockZkvmCryptoSpec,
    CelestiaNativeStorage,
>;
pub type DemoCelestiaRT = demo_stf::runtime::Runtime<CelestiaRollupSpec>;

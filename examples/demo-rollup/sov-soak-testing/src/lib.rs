use sov_address::MultiAddressEvm;
use sov_celestia_adapter::verifier::CelestiaSpec;
use sov_mock_da::{BlockProducingConfig, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_rollup_interface::execution_mode::Native;
pub use sov_soak_testing_lib::*;

pub const DEFAULT_BLOCK_TIME_MS: u64 = 200;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};

pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 5;

// Celestia
pub type CelestiaRollupSpec =
    ConfigurableSpec<CelestiaSpec, MockZkvm, MockZkvm, MultiAddressEvm, Native>;
pub type DemoCelestiaRT = demo_stf::runtime::Runtime<CelestiaRollupSpec>;

// Mock
pub type MockDemoRollupSpec =
    ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, MultiAddressEvm, Native>;
pub type DemoMockRT = demo_stf::runtime::Runtime<MockDemoRollupSpec>;

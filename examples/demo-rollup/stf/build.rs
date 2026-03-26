use demo_stf_declaration::MultiAddressEvmSolana;
use demo_stf_declaration::Runtime;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::execution_mode::Native;

type S = sov_modules_api::configurable_spec::ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
>;

fn main() -> anyhow::Result<()> {
    sov_build::Options::apply_defaults::<S, Runtime<S>>()
}

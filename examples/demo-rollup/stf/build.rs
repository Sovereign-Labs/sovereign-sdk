use demo_stf_declaration::MockDaSpec;
use demo_stf_declaration::MockZkvm;
use demo_stf_declaration::MultiAddressEvmSolana;
use demo_stf_declaration::Runtime;
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

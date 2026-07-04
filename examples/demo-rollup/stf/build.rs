use demo_stf_declaration::MultiAddressEvmSolana;
use demo_stf_declaration::Runtime;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;

#[cfg(feature = "native")]
type ExecMode = sov_modules_api::execution_mode::Native;

#[cfg(not(feature = "native"))]
type ExecMode = sov_modules_api::execution_mode::Zk;

type S = sov_modules_api::configurable_spec::ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    ExecMode,
>;

fn main() -> anyhow::Result<()> {
    // The chain identity is read here — in the leaf build crate — rather than
    // inside `sov-modules-api`, so editing chain-metadata.toml does not
    // rebuild the module system.
    sov_build::Options::apply_defaults::<S, Runtime<S>>(sov_build::ChainData {
        chain_id: sov_modules_api::macros::config_value!("CHAIN_ID"),
        chain_name: sov_modules_api::macros::config_value!("CHAIN_NAME").to_string(),
    })
}

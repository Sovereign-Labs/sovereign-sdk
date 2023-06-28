use glassbench::*;

use std::env;
use anyhow::Context;
use demo_stf::runner_config::from_toml_path;

use sov_demo_rollup::config::RollupConfig;
use sov_demo_rollup::rng_xfers::RngDaService;

use jupiter::verifier::address::CelestiaAddress;
use sov_modules_api::RpcRunner;
use sov_rollup_interface::mocks::TestBlob;


fn bench_rollup(bench: &mut Bench) -> Result<(), anyhow::Error> {
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
    let rollup_config: RollupConfig =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;

    Ok(())
}

glassbench!(
    "bench demo-rollup",
    bench_rollup,
);
use jupiter::da_service::DaServiceConfig;
use sov_stf_runner::RollupConfig;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    pub rollup_config: RollupConfig,
    pub da: DaServiceConfig,
}

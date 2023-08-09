use sov_stf_runner::RollupConfig;
use serde::Deserialize;

//TODO - replace with runtime config.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DaServiceConfig {
    pub light_client_url: String,
    pub node_client_url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    pub rollup_config: RollupConfig,
    pub sequencer_da_address: String,
    pub da: DaServiceConfig,
}

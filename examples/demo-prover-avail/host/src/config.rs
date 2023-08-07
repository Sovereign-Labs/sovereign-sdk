use demo_stf::runner_config::Config as RunnerConfig;
use serde::Deserialize;

//TODO - replace with runtime config.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DaServiceConfig {
    pub light_client_url: String,
    pub node_client_url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    pub start_height: u64,
    pub sequencer_da_address: String,
    pub da: DaServiceConfig,
    pub runner: RunnerConfig,
}

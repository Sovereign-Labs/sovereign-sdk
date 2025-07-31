use full_node_configs::runner::RollupConfig as RollupConfigBase;
pub use full_node_configs::runner::{
    from_toml_path, CorsConfiguration, HttpServerConfig, ProofManagerConfig, RunnerConfig,
};
pub use sov_metrics::{MonitoringConfig, TelegrafSocketConfig};

/// With sov-metrics
pub type RollupConfig<Address, Da> = RollupConfigBase<Address, Da, MonitoringConfig>;

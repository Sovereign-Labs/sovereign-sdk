//! This crate configures the da layer used by the rollup.
use sov_stf_runner::RollupConfig;
/// The type alias for the DA layer configuration. Change the contents of this alias if you change DA layers.
pub type DaConfig = sov_rollup_interface::mocks::MockDaConfig;
/// The type alias for the DA service. Change the contents of this alias if you change DA layers.
pub type DaService = sov_rollup_interface::mocks::MockDaService;

/// Creates a new instance of the DA Service
pub async fn start_da_service(rollup_config: &RollupConfig<DaConfig>) -> DaService {
    DaService::new(rollup_config.da.sender_address)
}

//! This crate configures the da layer used by the rollup. To switch to a
//! different DA layer:
//!   1. Switch the `sov_celestia_adapter` dependency in you Cargo.toml to the adapter for your chosen DA layer
//!   2. Update the `rollup_config.toml` to include the required configuration for your chosen DA layer
//!   3. Update the three `pub` declarations in this file for your DA layer
//!
//! Your rollup full node will automatically switch to the new DA layer.

/// The type alias for the DA layer configuration. Change the contents of this alias if you change DA layers.
pub type DaConfig = sov_rollup_interface::mocks::MockDaConfig;
/// The type alias for the DA layer verifier. Change the contents of this alias if you change DA layers.
pub type DaVerifier = MockDaVerifier;
/// The type alias for the DA service. Change the contents of this alias if you change DA layers.
pub type DaService = sov_rollup_interface::mocks::MockDaService;

use sov_rollup_interface::da::DaVerifier as _;
use sov_rollup_interface::mocks::MockDaVerifier;
use sov_stf_runner::RollupConfig;

/// Creates a new instance of the DA Service
pub async fn start_da_service(rollup_config: &RollupConfig<DaConfig>) -> DaService {
    DaService::new(rollup_config.da.sender_address)
}

/// Creates a new verifier for the rollup's DA.
pub fn new_da_verifier() -> DaVerifier {
    DaVerifier::new(())
}

//! This crate configures the da layer used by the rollup. To switch to a
//! different DA layer:
//!   1. Switch the `sov_celestia_adapter` dependency in you Cargo.toml to the adapter for your chosen DA layer
//!   2. Update the `rollup_config.toml` to include the required configuration for your chosen DA layer
//!   3. Update the three `pub` declarations in this file for your DA layer
//!
//! Your rollup full node will automatically switch to the new DA layer.

/// The type alias for the DA layer configuration. Change the contents of this alias if you change DA layers.
pub type DaConfig = sov_celestia_adapter::DaServiceConfig;
/// The type alias for the DA layer verifier. Change the contents of this alias if you change DA layers.
pub type DaVerifier = sov_celestia_adapter::verifier::CelestiaVerifier;
/// The type alias for the DA service. Change the contents of this alias if you change DA layers.
#[cfg(feature = "native")]
pub type DaService = sov_celestia_adapter::CelestiaService;

use sov_celestia_adapter::types::NamespaceId;
use sov_celestia_adapter::verifier::CelestiaVerifier;
#[cfg(feature = "native")]
use sov_celestia_adapter::verifier::RollupParams;
#[cfg(feature = "native")]
use sov_stf_runner::RollupConfig;

/// The Celestia namespace to which the rollup will write its data
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId([11; 8]);

/// Creates a new instance of the DA Service
#[cfg(feature = "native")]
pub async fn start_da_service(rollup_config: &RollupConfig<DaConfig>) -> DaService {
    DaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    )
    .await
}

/// Creates a new verifier for the rollup's DA.
pub fn new_da_verifier() -> DaVerifier {
    CelestiaVerifier {
        rollup_namespace: ROLLUP_NAMESPACE,
    }
}

//! A simple rollup that uses the Sovereign SDK.
//!
//! See the README for more information.
// TODO: #![doc = include_str!("../README.md")]
#![deny(missing_docs)]

use std::str::FromStr;

// Ensure exactly one ZKVM feature is enabled at the lib level
#[cfg(all(feature = "mock_zkvm", feature = "risc0"))]
compile_error!("Both mock_zkvm and risc0 are enabled. Enable exactly one ZKVM feature.");
#[cfg(all(feature = "mock_zkvm", feature = "sp1"))]
compile_error!("Both mock_zkvm and sp1 are enabled. Enable exactly one ZKVM feature.");
#[cfg(all(feature = "risc0", feature = "sp1"))]
compile_error!("Both risc0 and sp1 are enabled. Enable exactly one ZKVM feature.");

// Ensure exactly one DA feature is enabled
#[cfg(all(feature = "mock_da", feature = "celestia_da"))]
compile_error!("Both mock_da and celestia_da are enabled. Enable exactly one DA feature.");
#[cfg(all(feature = "mock_da", feature = "mock_da_external"))]
compile_error!("Both mock_da and mock_da_external are enabled. Enable exactly one DA feature.");
#[cfg(all(feature = "mock_da_external", feature = "celestia_da"))]
compile_error!("Both mock_da_external and celestia_da are enabled. Enable exactly one DA feature.");

#[cfg(feature = "mock_da")]
mod mock_rollup;
#[cfg(feature = "mock_da")]
pub use mock_rollup::*;

#[cfg(feature = "celestia_da")]
mod celestia_rollup;
#[cfg(feature = "celestia_da")]
pub use celestia_rollup::*;

#[cfg(feature = "mock_da_external")]
mod external_mock_rollup;
#[cfg(feature = "mock_da_external")]
pub use external_mock_rollup::*;

mod solana_offchain_endpoint;

/// Feature-gated inner ZKVM selection.
pub mod zk;
pub use zk::*;

/// The rollup stores its data in the namespace b"sov-test" on Celestia
/// You can change this constant by modifying BATCH_NAMESPACE in constants.toml
#[cfg(feature = "celestia_da")]
pub const ROLLUP_BATCH_NAMESPACE: sov_celestia_adapter::types::Namespace =
    sov_celestia_adapter::types::Namespace::const_v0(
        sov_modules_api::macros::config_value!("BATCH_NAMESPACE"),
    );

/// The rollup stores the zk proofs in the namespace b"sov-test-p" on Celestia.
/// You can change this constant by modifying PROOF_NAMESPACE in constants.toml
#[cfg(feature = "celestia_da")]
pub const ROLLUP_PROOF_NAMESPACE: sov_celestia_adapter::types::Namespace =
    sov_celestia_adapter::types::Namespace::const_v0(
        sov_modules_api::macros::config_value!("PROOF_NAMESPACE"),
    );

fn sequencer_type(
    config: &sov_full_node_configs::sequencer::SequencerConfig<impl Copy>,
) -> sov_modules_api::SequencerType {
    if config.is_preferred_sequencer() {
        sov_modules_api::SequencerType::Preferred
    } else {
        sov_modules_api::SequencerType::NonPreferred
    }
}

// TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/387
fn eth_dev_signer() -> sov_ethereum::Signers {
    sov_ethereum::Signers::new(vec![secp256k1::SecretKey::from_str(
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    )
    .unwrap()])
}

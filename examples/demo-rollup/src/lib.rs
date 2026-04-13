//! A simple rollup that uses the Sovereign SDK.
//!
//! See the README for more information.
// TODO: #![doc = include_str!("../README.md")]
#![deny(missing_docs)]

use std::str::FromStr;

// DA rollup modules compile when their feature is enabled.
// mock_da compiles both in-process and external (RPC) mock variants;
// the binary selects at runtime via --external.

#[cfg(feature = "mock_da")]
mod mock_rollup;
#[cfg(feature = "mock_da")]
pub use mock_rollup::*;

#[cfg(feature = "mock_da")]
mod external_mock_rollup;
#[cfg(feature = "mock_da")]
pub use external_mock_rollup::*;

#[cfg(feature = "celestia_da")]
mod celestia_rollup;
#[cfg(feature = "celestia_da")]
pub use celestia_rollup::*;

mod solana_offchain_endpoint;

/// Feature-gated inner ZKVM selection.
pub mod zk;
pub use zk::*;

/// The rollup stores its data in the namespace b"sov-test" on Celestia
/// You can change this constant by modifying BATCH_NAMESPACE in constants.toml
#[cfg(feature = "celestia_da")]
pub const ROLLUP_BATCH_NAMESPACE: sov_celestia_adapter::types::Namespace =
    sov_celestia_adapter::types::Namespace::const_v0(sov_modules_api::macros::config_value!(
        "BATCH_NAMESPACE"
    ));

/// The rollup stores the zk proofs in the namespace b"sov-test-p" on Celestia.
/// You can change this constant by modifying PROOF_NAMESPACE in constants.toml
#[cfg(feature = "celestia_da")]
pub const ROLLUP_PROOF_NAMESPACE: sov_celestia_adapter::types::Namespace =
    sov_celestia_adapter::types::Namespace::const_v0(sov_modules_api::macros::config_value!(
        "PROOF_NAMESPACE"
    ));

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

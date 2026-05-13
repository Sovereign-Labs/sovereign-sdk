//! A simple rollup that uses the Sovereign SDK.
//!
//! See the README for more information.
// TODO: #![doc = include_str!("../README.md")]
#![deny(missing_docs)]
use sov_celestia_adapter::types::Namespace;
use sov_modules_api::macros::config_value;
use std::str::FromStr;
mod aggregated_proof;
pub use aggregated_proof::read_latest_aggregated_proof;
mod mock_helper;
pub use mock_helper::{
    create_mock_prover_service, read_mock_code_commitments_from_env, set_inner_code_commitment_env,
    set_outer_code_commitment_env, Hasher, MockAggregatedProofPublicData, NativeStorage,
};
mod mock_rollup;
pub use mock_rollup::*;
mod mock_sp1_rollup;
pub use mock_sp1_rollup::*;
mod chain_state_override;
pub use chain_state_override::override_code_commitments_in_chain_state;
mod celestia_rollup;
pub use celestia_rollup::*;
mod external_mock_rollup;
pub use external_mock_rollup::*;
mod solana_offchain_endpoint;

/// The rollup stores its data in the namespace b"sov-test" on Celestia
/// You can change this constant by modifying BATCH_NAMESPACE in constants.toml
pub const ROLLUP_BATCH_NAMESPACE: Namespace = Namespace::const_v0(config_value!("BATCH_NAMESPACE"));
/// The rollup stores the zk proofs in the namespace b"sov-test-p" on Celestia.
/// You can change this constant by modifying PROOF_NAMESPACE in constants.toml
pub const ROLLUP_PROOF_NAMESPACE: Namespace = Namespace::const_v0(config_value!("PROOF_NAMESPACE"));

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

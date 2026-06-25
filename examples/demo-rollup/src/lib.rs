//! A simple rollup that uses the Sovereign SDK.
//!
//! See the README for more information.
// TODO: #![doc = include_str!("../README.md")]
#![deny(missing_docs)]

#[cfg(not(any(feature = "mock_da", feature = "celestia_da")))]
compile_error!("enable at least one DA feature: `mock_da` or `celestia_da`");

use std::str::FromStr;

mod chain_state_override;
pub use chain_state_override::override_code_commitments_in_chain_state;
mod solana_offchain_endpoint;

// Mock-DA rollups: both MockZkvm and SP1 inner VMs (zkVM chosen at runtime via `--zk-vm`).
#[cfg(feature = "mock_da")]
mod aggregated_proof;
#[cfg(feature = "mock_da")]
pub use aggregated_proof::read_latest_aggregated_proof;
#[cfg(feature = "mock_da")]
mod mock_helper;
#[cfg(feature = "mock_da")]
pub use mock_helper::{
    create_mock_prover_service, read_mock_code_commitments_from_env, set_inner_code_commitment_env,
    set_outer_code_commitment_env, Hasher, MockAggregatedProofPublicData, NativeStorage,
};
#[cfg(feature = "mock_da")]
mod mock_rollup;
#[cfg(feature = "mock_da")]
pub use mock_rollup::*;
#[cfg(feature = "mock_da")]
mod mock_sp1_rollup;
#[cfg(feature = "mock_da")]
mod sp1_helper;
#[cfg(feature = "mock_da")]
pub use mock_sp1_rollup::*;
#[cfg(feature = "mock_da")]
mod external_mock_sp1_rollup;
#[cfg(feature = "mock_da")]
pub use external_mock_sp1_rollup::*;
#[cfg(feature = "mock_da")]
mod external_mock_rollup;
#[cfg(feature = "mock_da")]
pub use external_mock_rollup::*;

// Celestia-DA rollup: Risc0 inner VM + MockZkvm outer VM.
#[cfg(feature = "celestia_da")]
mod celestia_rollup;
#[cfg(feature = "celestia_da")]
pub use celestia_rollup::*;

/// The zkVM a rollup runs on. Selected at runtime via the `--zk-vm` flag and
/// shared by both the node binary (which picks the rollup to run) and the
/// light-client binary (which picks the proof verifier).
#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum SupportedZkVm {
    /// The mock zkVM used for fast, proof-free local development.
    Mock,
    /// The SP1 zkVM.
    Sp1,
}

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

// Imports used only by `get_persisted_outer_proof`; gated with it behind `mock_da`
// (only the mock/sp1 prover helpers reconcile a persisted outer proof).
#[cfg(feature = "mock_da")]
use serde::de::DeserializeOwned;
#[cfg(feature = "mock_da")]
use sov_db::ledger_db::LedgerDb;
#[cfg(feature = "mock_da")]
use sov_rollup_interface::common::SlotNumber;
#[cfg(feature = "mock_da")]
use sov_rollup_interface::da::DaSpec;
#[cfg(feature = "mock_da")]
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, CodeCommitmentHash, SerializedAggregatedProof,
};
#[cfg(feature = "mock_da")]
use sov_rollup_interface::zk::ZkVerifier;

/// Loads the latest persisted aggregated proof and reconciles it with the
/// current inner/outer code commitments to decide what the outer prover should
/// resume from.
#[cfg(feature = "mock_da")]
pub(crate) async fn get_persisted_outer_proof<Vm, Address, Da, Root>(
    ledger_db: &LedgerDb,
    inner_code_commitment_hash: CodeCommitmentHash,
    outer_code_commitment_hash: CodeCommitmentHash,
    start_fresh_outer_proof_on_resync: bool,
) -> anyhow::Result<(Option<SerializedAggregatedProof>, Option<SlotNumber>)>
where
    Vm: ZkVerifier,
    Da: DaSpec,
    AggregatedProofPublicData<Address, Da, Root>: DeserializeOwned,
{
    let previous_proof = read_latest_aggregated_proof(ledger_db).await;

    let previous_public_data = previous_proof
        .as_ref()
        .map(|proof| {
            Vm::extract_public_data::<AggregatedProofPublicData<Address, Da, Root>>(
                &proof.clone().to_serialized_zk_proof(),
            )
        })
        .transpose()
        .map_err(|e| {
            anyhow::anyhow!("Failed to extract public data from persisted aggregated proof: {e:?}")
        })?;

    let latest_proof_final_slot = previous_public_data.as_ref().map(|p| p.final_slot_number);

    if let (false, Some(prev)) = (
        start_fresh_outer_proof_on_resync,
        previous_public_data.as_ref(),
    ) {
        anyhow::ensure!(
            inner_code_commitment_hash == prev.inner_vkey_hash,
            "inner code commitment changed since last proof; pass start_fresh_outer_proof_on_resync to reset"
        );
        anyhow::ensure!(
            outer_code_commitment_hash == prev.outer_vk_hash,
            "outer code commitment changed since last proof; pass start_fresh_outer_proof_on_resync to reset"
        );
    }

    let previous_outer_proof = if start_fresh_outer_proof_on_resync {
        None
    } else {
        previous_proof
    };

    Ok((previous_outer_proof, latest_proof_final_slot))
}

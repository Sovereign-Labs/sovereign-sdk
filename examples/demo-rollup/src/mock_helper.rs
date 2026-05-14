//! Helpers shared between the mock-da-backed demo rollups
//! ([`crate::MockDemoRollup`] and [`crate::ExternalMockDemoRollup`]).
//!
//! Centralizes the bits their `create_prover_service` / `compute_code_commitments`
//! impls have in common: env-driven code commitments, the `Hasher` / `NativeStorage`
//! type aliases, and assembly of the [`ParallelProverService`].

use demo_stf::MultiAddressEvmSolana;
use sov_db::ledger_db::LedgerDb;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::{
    MockCodeCommitment, MockZkVerifier, MockZkvm, MockZkvmCryptoSpec, MockZkvmHost,
};
use sov_modules_api::{CodeCommitmentTrait, CryptoSpec};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData;
use sov_rollup_interface::zk::ZkVerifier;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{DefaultStorageSpec, Storage};
use sov_stf_runner::processes::ParallelProverService;
use sov_stf_runner::RollupConfig;

use crate::read_latest_aggregated_proof;

/// Hasher used by mock-da-backed demo rollups.
pub type Hasher = <MockZkvmCryptoSpec as CryptoSpec>::Hasher;

/// Native storage type for mock-da-backed demo rollups.
pub type NativeStorage =
    NomtProverStorage<DefaultStorageSpec<Hasher>, <MockDaSpec as DaSpec>::SlotHash>;

/// Aggregated-proof public data shape shared by the mock-da-backed demo rollups.
pub type MockAggregatedProofPublicData =
    AggregatedProofPublicData<MultiAddressEvmSolana, MockDaSpec, <NativeStorage as Storage>::Root>;

/// [`ParallelProverService`] shape shared by the mock-da-backed demo rollups,
/// parameterized only over the concrete DA service.
pub type MockParallelProverService<Da> = ParallelProverService<
    MultiAddressEvmSolana,
    <NativeStorage as Storage>::Root,
    <NativeStorage as Storage>::Witness,
    Da,
    MockZkvm,
    MockZkvm,
>;

/// Env var carrying the inner mock code commitment.
const MOCK_INNER_CODE_COMMITMENT_ENV: &str = "SOV_MOCK_INNER_CODE_COMMITMENT";

/// Env var carrying the outer mock code commitment.
const MOCK_OUTER_CODE_COMMITMENT_ENV: &str = "SOV_MOCK_OUTER_CODE_COMMITMENT";

/// Reads inner and outer mock code commitments from
/// [`MOCK_INNER_CODE_COMMITMENT_ENV`] and [`MOCK_OUTER_CODE_COMMITMENT_ENV`],
/// falling back to [`MockCodeCommitment::default`] when unset.
pub fn read_mock_code_commitments_from_env() -> (MockCodeCommitment, MockCodeCommitment) {
    let inner = mock_code_commitment_from_env(MOCK_INNER_CODE_COMMITMENT_ENV).unwrap_or_default();
    let outer = mock_code_commitment_from_env(MOCK_OUTER_CODE_COMMITMENT_ENV).unwrap_or_default();
    (inner, outer)
}

/// Test-only helper: sets [`MOCK_INNER_CODE_COMMITMENT_ENV`].
pub fn set_inner_code_commitment_env(commitment: MockCodeCommitment) {
    std::env::set_var(MOCK_INNER_CODE_COMMITMENT_ENV, commitment_hex(&commitment));
}

/// Test-only helper: sets [`MOCK_OUTER_CODE_COMMITMENT_ENV`].
pub fn set_outer_code_commitment_env(commitment: MockCodeCommitment) {
    std::env::set_var(MOCK_OUTER_CODE_COMMITMENT_ENV, commitment_hex(&commitment));
}

fn commitment_hex(commitment: &MockCodeCommitment) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(commitment.0.len() * 2);
    for byte in &commitment.0 {
        write!(&mut out, "{byte:02x}").expect("writing to String never fails");
    }
    out
}

/// Reads a [`MockCodeCommitment`] from `env_var`. Assumes the value was
/// written by [`commitment_hex`] (16 lowercase hex chars, no prefix). Returns
/// `None` if the variable is unset; panics if set but unparseable.
fn mock_code_commitment_from_env(env_var: &str) -> Option<MockCodeCommitment> {
    let raw = std::env::var(env_var).ok()?;
    let n = u64::from_str_radix(&raw, 16).unwrap_or_else(|e| {
        panic!("{env_var}: expected output of commitment_hex, got {raw:?}: {e}")
    });
    Some(MockCodeCommitment(n.to_be_bytes()))
}

/// Shared `create_prover_service` body for the mock-da-backed demo rollups.
pub async fn create_mock_prover_service<Da>(
    rollup_config: &RollupConfig<MultiAddressEvmSolana, Da>,
    ledger_db: &LedgerDb,
    start_fresh_outer_proof_on_resync: bool,
) -> anyhow::Result<(MockParallelProverService<Da>, Option<SlotNumber>)>
where
    Da: DaService,
    Da::Verifier: Default,
{
    let (inner_code_commitment, outer_code_commitment) = read_mock_code_commitments_from_env();

    // Read the persisted proof.
    let previous_proof = read_latest_aggregated_proof(ledger_db).await;
    let previous_public_data: Option<MockAggregatedProofPublicData> =
        match previous_proof.as_ref() {
            None => None,
            Some(proof) => Some(
                MockZkVerifier::extract_public_data(&proof.clone().to_serialized_zk_proof())
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to extract public data from persisted aggregated proof: {e}"
                        )
                    })?,
            ),
        };

    let latest_proof_final_slot = previous_public_data.as_ref().map(|p| p.final_slot_number);

    if let (false, Some(prev)) = (
        start_fresh_outer_proof_on_resync,
        previous_public_data.as_ref(),
    ) {
        anyhow::ensure!(
            inner_code_commitment.to_hash() == prev.inner_vkey_hash,
            "inner code commitment changed since last proof; pass start_fresh_outer_proof_on_resync to reset"
        );
        anyhow::ensure!(
            outer_code_commitment.to_hash() == prev.outer_vk_hash,
            "outer code commitment changed since last proof; pass start_fresh_outer_proof_on_resync to reset"
        );
    }

    let previous_outer_proof = if start_fresh_outer_proof_on_resync {
        None
    } else {
        previous_proof
    };

    let inner_vm = MockZkvmHost::new_non_blocking().with_code_commitment(inner_code_commitment);

    let outer_vm = MockZkvmHost::new_non_blocking_with_previous_outer_proof(previous_outer_proof)
        .with_code_commitment(outer_code_commitment);
    let prover = ParallelProverService::new_with_default_workers(
        inner_vm,
        outer_vm,
        Da::Verifier::default(),
        rollup_config.proof_manager.prover_address,
        rollup_config.proof_manager.prover_thread_count(),
    );

    Ok((prover, latest_proof_final_slot))
}

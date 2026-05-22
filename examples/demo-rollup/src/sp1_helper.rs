//! Shared SP1 setup and prover-service construction for the SP1 demo rollups.

use std::sync::Arc;

use demo_stf::MultiAddressEvmSolana;
use sov_db::ledger_db::LedgerDb;
use sov_mock_da::MockDaSpec;
use sov_modules_api::{CodeCommitmentTrait, CryptoSpec};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_sp1_adapter::host::{code_commitment_from_verifying_key, SP1AggregationHost, SP1Host};
use sov_sp1_adapter::{SP1CryptoSpec, SP1MethodId, SP1Verifier, SP1};
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{DefaultStorageSpec, Storage};
use sov_stf_runner::processes::ParallelProverService;
use sov_stf_runner::RollupConfig;

use crate::get_persisted_outer_proof;

pub(crate) type Sp1Hasher = <SP1CryptoSpec as CryptoSpec>::Hasher;
pub(crate) type Sp1NativeStorage =
    NomtProverStorage<DefaultStorageSpec<Sp1Hasher>, <MockDaSpec as DaSpec>::SlotHash>;

/// Output of [`build_sp1_inner_host`]: the inner [`SP1Host`] together with the
/// inner and outer code commitments derived from the same setup pass.
pub(crate) struct Sp1InnerHostBundle {
    pub(crate) inner_host: SP1Host,
    pub(crate) inner_code_commitment: SP1MethodId,
    pub(crate) outer_code_commitment: SP1MethodId,
}

/// Runs the SP1 prover setup once per ELF and returns the inner host plus both
/// code commitments. Callers that only need the commitments can drop
/// `inner_host`; the setup cost is identical either way.
pub(crate) fn build_sp1_inner_host(
    inner_elf: &[u8],
    outer_elf: &[u8],
) -> anyhow::Result<Sp1InnerHostBundle> {
    let outer_vk = Arc::new(sov_sp1_adapter::host::verifying_key_from_elf(outer_elf)?);
    let outer_code_commitment = code_commitment_from_verifying_key(&outer_vk);
    let inner_host = SP1Host::new(inner_elf, outer_vk)?;
    let inner_code_commitment = code_commitment_from_verifying_key(inner_host.verifying_key());
    Ok(Sp1InnerHostBundle {
        inner_host,
        inner_code_commitment,
        outer_code_commitment,
    })
}

/// Concrete prover-service type used by both SP1 mock rollup variants.
pub(crate) type Sp1ProverService<Da> = ParallelProverService<
    MultiAddressEvmSolana,
    <Sp1NativeStorage as Storage>::Root,
    <Sp1NativeStorage as Storage>::Witness,
    Da,
    SP1,
    SP1,
>;

/// Shared body of `FullNodeBlueprint::create_prover_service` for the SP1 mock
/// rollups. Builds the inner+outer SP1 hosts off the async runtime, reconciles
/// the persisted outer proof, and constructs the [`ParallelProverService`].
pub(crate) async fn create_sp1_prover_service<Da>(
    rollup_config: &RollupConfig<MultiAddressEvmSolana, Da>,
    ledger_db: &LedgerDb,
    start_fresh_outer_proof_on_resync: bool,
) -> anyhow::Result<(Sp1ProverService<Da>, Option<SlotNumber>)>
where
    Da: DaService<Spec = MockDaSpec>,
    Da::Verifier: Default,
{
    let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
    let agg_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;

    // SP1's blocking CPU prover spins up its own tokio runtime during setup,
    // so it must be constructed off the async executor thread.
    let Sp1InnerHostBundle {
        inner_host: inner_vm,
        inner_code_commitment,
        outer_code_commitment,
    } = tokio::task::spawn_blocking(move || build_sp1_inner_host(elf, agg_elf))
        .await
        .expect("SP1 host setup task panicked")?;

    let inner_verifying_key = inner_vm.verifying_key().clone();

    let (previous_for_outer, latest_proof_final_slot) = get_persisted_outer_proof::<
        SP1Verifier,
        MultiAddressEvmSolana,
        MockDaSpec,
        <Sp1NativeStorage as Storage>::Root,
    >(
        ledger_db,
        inner_code_commitment.to_hash(),
        outer_code_commitment.to_hash(),
        start_fresh_outer_proof_on_resync,
    )
    .await?;

    let outer_vm = tokio::task::spawn_blocking(move || {
        SP1AggregationHost::new_with_previous_proof(
            agg_elf,
            inner_verifying_key,
            previous_for_outer,
        )
        .expect("Failed to create SP1AggregationHost from aggregation guest ELF")
    })
    .await
    .expect("SP1AggregationHost setup task panicked");

    let da_verifier = Default::default();

    let proof_manager = rollup_config
        .proof_manager
        .as_ref()
        .expect("proof_manager must be set when prover is enabled");
    let num_threads = proof_manager.prover_thread_count();
    let prover = ParallelProverService::new_with_default_workers(
        inner_vm,
        outer_vm,
        da_verifier,
        proof_manager.prover_address,
        num_threads,
    );

    Ok((prover, latest_proof_final_slot))
}

/// Shared body of `FullNodeBlueprint::compute_code_commitments` for the SP1
/// mock rollups.
pub(crate) fn compute_sp1_code_commitments() -> anyhow::Result<(SP1MethodId, SP1MethodId)> {
    let inner_elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
    let outer_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;

    let Sp1InnerHostBundle {
        inner_code_commitment,
        outer_code_commitment,
        ..
    } = build_sp1_inner_host(inner_elf, outer_elf)?;

    Ok((inner_code_commitment, outer_code_commitment))
}

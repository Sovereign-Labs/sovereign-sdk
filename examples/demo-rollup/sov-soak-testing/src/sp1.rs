use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use demo_stf::genesis_config::GenesisPaths;
use demo_stf::MultiAddressEvmSolana;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockDaSpec};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::FullNodeBlueprint;
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::CryptoSpec;
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_sp1_adapter::host::{SP1AggregationHost, SP1Host};
use sov_sp1_adapter::{SP1MethodId, SP1};
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;
use sov_state::Storage;
use sov_stf_runner::processes::ParallelProverService;
pub use sov_stf_runner::processes::RollupProverConfig;
use sov_stf_runner::RollupConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath};
use sov_test_utils::{ProverFactory, RtAgnosticBlueprint};

use crate::{make_postgres_config, DEFAULT_FINALIZATION_BLOCKS};

type SP1Hasher = <sov_sp1_adapter::SP1CryptoSpec as CryptoSpec>::Hasher;

type SP1NativeStorage =
    NomtProverStorage<DefaultStorageSpec<SP1Hasher>, <MockDaSpec as DaSpec>::SlotHash>;
pub type SP1Spec = ConfigurableSpec<
    MockDaSpec,
    SP1,
    SP1,
    MultiAddressEvmSolana,
    Native,
    sov_sp1_adapter::SP1CryptoSpec,
    SP1NativeStorage,
>;
pub type SP1RT = demo_stf::runtime::Runtime<SP1Spec>;

/// Prover factory backed by [`ParallelProverService`] with `SP1Host` for the inner.
///
/// The SP1 SDK's prover backend is selected via the `SP1_PROVER` environment
/// variable read by `sp1_sdk` at runtime — `cpu`, `cuda`, `mock`, or `network`.
/// For network proving, set `SP1_PROVER=network` along with `NETWORK_PRIVATE_KEY`
/// before launching the rollup.
pub struct ParallelProverFactory;

#[async_trait]
impl ProverFactory<SP1Spec> for ParallelProverFactory {
    type ProverService = ParallelProverService<
        <SP1Spec as Spec>::Address,
        <<SP1Spec as Spec>::Storage as Storage>::Root,
        <<SP1Spec as Spec>::Storage as Storage>::Witness,
        StorableMockDaService,
        SP1,
        SP1,
    >;

    async fn create(
        _prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<<SP1Spec as Spec>::Address, StorableMockDaService>,
    ) -> Self::ProverService {
        let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
        let agg_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;
        assert!(
            !elf.is_empty(),
            "SP1 guest ELF is empty — build it first (cd provers/sp1 && cargo build)"
        );
        assert!(
            !agg_elf.is_empty(),
            "SP1 aggregation guest ELF is empty — build it first (cd provers/sp1 && cargo build)"
        );

        // SP1 host setup spins up its own runtime; do it off the async executor.
        let outer_verifying_key = tokio::task::spawn_blocking(move || {
            sov_sp1_adapter::host::verifying_key_from_elf(agg_elf).map(Arc::new)
        })
        .await
        .expect("Outer verifying key setup task panicked")
        .expect("Failed to derive outer verifying key from aggregation guest ELF");

        let inner_vm = tokio::task::spawn_blocking(move || SP1Host::new(elf, outer_verifying_key))
            .await
            .expect("SP1Host setup task panicked")
            .expect("Failed to create SP1Host from guest ELF");

        let inner_verifying_key = inner_vm.verifying_key().clone();

        let outer_vm = tokio::task::spawn_blocking(move || {
            SP1AggregationHost::new(agg_elf, inner_verifying_key)
                .expect("Failed to create SP1AggregationHost from aggregation guest ELF")
        })
        .await
        .expect("SP1AggregationHost setup task panicked");

        let da_verifier = Default::default();

        ParallelProverService::new_with_default_workers(
            inner_vm,
            outer_vm,
            da_verifier,
            rollup_config.proof_manager.prover_address,
        )
    }

    fn code_commitments() -> anyhow::Result<(SP1MethodId, SP1MethodId)> {
        let inner_elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
        let outer_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;
        let inner = sov_sp1_adapter::host::code_commitment_from_elf(inner_elf)?;
        let outer = sov_sp1_adapter::host::code_commitment_from_elf(outer_elf)?;
        Ok((inner, outer))
    }
}

pub type NetworkProvingBlueprint = RtAgnosticBlueprint<
    SP1Spec,
    SP1RT,
    sov_db::storage_manager::NomtStorageManager<MockDaSpec, SP1Hasher, SP1NativeStorage>,
    ParallelProverFactory,
>;

fn sp1_genesis_paths() -> GenesisPaths {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let dir = Path::new(manifest_dir).join("../../test-data/genesis/integration-tests");
    let mut paths = GenesisPaths::from_dir(&dir);
    paths.chain_state_genesis_path = dir.join("chain_state_zk.json");
    paths.paymaster_genesis_path = dir.join("paymaster_with_payer.json");
    paths
}

pub async fn create_sp1_rollup_builder(
    storage_path: PathBuf,
    axum_port: u16,
    db_connection_url: Option<String>,
) -> RollupBuilder<NetworkProvingBlueprint> {
    let mut genesis_config =
        demo_stf::genesis_config::create_genesis_config::<SP1Spec>(&sp1_genesis_paths())
            .expect("Failed to create demo-stf genesis config");

    // The shared `chain_state_zk.json` ships with placeholder-zero code commitments.
    // Real SP1 proofs commit to non-zero `SP1MethodId`s derived from the guest ELFs,
    // so we override the in-memory genesis commitments with the actual ELF-derived
    // values before constructing the rollup. The on-disk JSON file is left untouched.
    let (inner, outer) =
        tokio::task::spawn_blocking(NetworkProvingBlueprint::compute_code_commitments)
            .await
            .expect("Code-commitment computation task panicked")
            .expect("Failed to compute code commitments for the SP1 soak rollup");
    genesis_config.chain_state.inner_code_commitment = inner;
    genesis_config.chain_state.outer_code_commitment = outer;

    let postgres_config = make_postgres_config(db_connection_url);

    // SP1 network proving takes ~10s per proof. With aggregated_proof_block_jump=8,
    // each batch takes ~80s. Use 10s block time so the prover can keep pace with DA.
    let block_producing_config = BlockProducingConfig::Periodic {
        block_time_ms: 10_000,
    };

    RollupBuilder::<NetworkProvingBlueprint>::new_with_storage_path(
        GenesisSource::CustomParams(GenesisParams {
            runtime: genesis_config,
        }),
        block_producing_config,
        DEFAULT_FINALIZATION_BLOCKS,
        StoragePath::Buf(storage_path),
        false,
    )
    .set_config(|config| {
        config.telegraf_address = sov_metrics::MonitoringConfig::standard().telegraf_address;
        config.automatic_batch_production = true;
        config.sequencer_config = SequencerKindConfig::Preferred(PreferredSequencerConfig {
            minimum_profit_per_tx: 0,
            postgres_config,
            batch_execution_time_limit_millis: 11_000,
            disable_state_root_consistency_checks: true,
            // Pinned cache is currently not compatible with ZKPs
            num_cache_warmup_workers: 0,
            ..Default::default()
        });
        config.aggregated_proof_block_jump = 8;
        config.axum_port = axum_port;
        // With 10s block time and finalization_blocks=5, DA finalization takes ~50s.
        // The default 60s is too tight — bump to 120s to avoid spurious timeouts.
        config.blob_processing_timeout_secs = 120;
    })
}

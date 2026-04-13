use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use demo_stf::genesis_config::GenesisPaths;
use demo_stf::MultiAddressEvmSolana;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockDaSpec};
use sov_mock_zkvm::{MockZkvm, MockZkvmNetwork};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Spec;
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::CryptoSpec;
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_sp1_adapter::network::SP1Network;
use sov_sp1_adapter::SP1;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;
use sov_state::Storage;
use sov_stf_runner::processes::NetworkProverService;
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
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
    sov_sp1_adapter::SP1CryptoSpec,
    SP1NativeStorage,
>;
pub type SP1RT = demo_stf::runtime::Runtime<SP1Spec>;

/// Prover factory that submits proofs to the SP1 Succinct proving network.
pub struct NetworkProverFactory;

#[async_trait]
impl ProverFactory<SP1Spec> for NetworkProverFactory {
    type ProverService = NetworkProverService<
        <SP1Spec as Spec>::Address,
        <<SP1Spec as Spec>::Storage as Storage>::Root,
        <<SP1Spec as Spec>::Storage as Storage>::Witness,
        StorableMockDaService,
        SP1,
        MockZkvm,
    >;

    async fn create(
        _prover_config: RollupProverConfig<SP1>,
        rollup_config: &RollupConfig<<SP1Spec as Spec>::Address, StorableMockDaService>,
    ) -> Self::ProverService {
        let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
        assert!(
            !elf.is_empty(),
            "SP1 guest ELF is empty — build it first (cd provers/sp1 && cargo build)"
        );

        let inner_vm = SP1Network::new(elf)
            .await
            .expect("Failed to create SP1Network — is NETWORK_PRIVATE_KEY set?");
        // auto-complete outer proofs — real outer not supported yet
        let outer_vm = MockZkvmNetwork::new(true);

        NetworkProverService::new(
            inner_vm,
            outer_vm,
            Default::default(),
            rollup_config.proof_manager.prover_address,
            Duration::from_secs(600),
        )
    }
}

pub type NetworkProvingBlueprint = RtAgnosticBlueprint<
    SP1Spec,
    SP1RT,
    sov_db::storage_manager::NomtStorageManager<MockDaSpec, SP1Hasher, SP1NativeStorage>,
    NetworkProverFactory,
>;

fn sp1_genesis_paths() -> GenesisPaths {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let dir = Path::new(manifest_dir).join("../../test-data/genesis/integration-tests");
    let mut paths = GenesisPaths::from_dir(&dir);
    paths.chain_state_genesis_path = dir.join("chain_state_zk.json");
    paths.paymaster_genesis_path = dir.join("paymaster_with_payer.json");
    paths
}

pub fn create_sp1_rollup_builder(
    storage_path: PathBuf,
    axum_port: u16,
    db_connection_url: Option<String>,
) -> RollupBuilder<NetworkProvingBlueprint> {
    let genesis_config =
        demo_stf::genesis_config::create_genesis_config::<SP1Spec>(&sp1_genesis_paths())
            .expect("Failed to create demo-stf genesis config");
    let postgres_config = make_postgres_config(db_connection_url);

    // SP1 network proving takes ~10s per proof. With aggregated_proof_block_jump=3,
    // each batch takes ~30s. Use 10s block time so the prover can keep pace with DA.
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
            ..Default::default()
        });
        config.aggregated_proof_block_jump = 3;
        config.axum_port = axum_port;
        // With 10s block time and finalization_blocks=5, DA finalization takes ~50s.
        // The default 60s is too tight — bump to 120s to avoid spurious timeouts.
        config.blob_processing_timeout_secs = 120;
    })
}

use std::path::PathBuf;

use demo_stf::MultiAddressEvmSolana;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath};
use sov_test_utils::{RtAgnosticBlueprint, TestSpec};

use crate::{
    make_postgres_config, Setup, SoakHasher, TestRT, DEFAULT_BLOCK_PRODUCING_CONFIG,
    DEFAULT_FINALIZATION_BLOCKS,
};

type MockNativeStorage =
    NomtProverStorage<DefaultStorageSpec<SoakHasher>, <MockDaSpec as DaSpec>::SlotHash>;
pub type MockDemoRollupSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
    MockZkvmCryptoSpec,
    MockNativeStorage,
>;
pub type DemoMockRT = demo_stf::runtime::Runtime<MockDemoRollupSpec>;

pub type MockRollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRT>;

pub fn create_mock_rollup_builder(
    storage_path: PathBuf,
    axum_port: u16,
    setup: &Setup,
    db_connection_url: Option<String>,
) -> RollupBuilder<MockRollupBlueprint> {
    let postgres_config = make_postgres_config(db_connection_url);
    let da_address = setup.sequencer.da_address;

    RollupBuilder::<MockRollupBlueprint>::new_with_storage_path(
        GenesisSource::CustomParams(setup.genesis_config.clone().into_genesis_params()),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
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
            batch_execution_time_limit_millis: 400,
            ..Default::default()
        });
        config.prover_address = setup.prover.user_info.address().to_string();
        config.aggregated_proof_block_jump = 3;
        config.axum_port = axum_port;
    })
    .set_da_config(|da_config| {
        da_config.sender_address = da_address;
    })
}

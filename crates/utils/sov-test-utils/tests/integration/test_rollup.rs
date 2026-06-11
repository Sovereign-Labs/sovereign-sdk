use std::sync::Arc;

use sov_db::ledger_db::LedgerDb;
use sov_mock_da::MockBlock;
use sov_modules_api::Runtime;
use sov_modules_rollup_blueprint::logging::initialize_logging;
use sov_modules_rollup_blueprint::FullNodeBlueprint;
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder};
use sov_test_utils::{
    generate_optimistic_runtime, RtAgnosticBlueprint, TestSpec,
    TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
};
use tempfile::TempDir;

generate_optimistic_runtime!(TestRuntime <=);

type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

#[tokio::test(flavor = "multi_thread")]
async fn validates_rollup_mode_before_genesis_initialization() {
    let _guard = initialize_logging();

    let dir = Arc::new(tempfile::tempdir().unwrap());
    let genesis_params = GenesisParams {
        runtime: <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            HighLevelOptimisticGenesisConfig::generate().into(),
        ),
    };

    let builder = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params.clone()),
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        1,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir);
        c.rollup_prover_config = RollupProverConfig::Disabled;
    })
    .with_standard_sequencer();
    let mut rollup_config = builder.rollup_config();
    rollup_config.proof_manager = None;

    let blueprint = TestBlueprint::default();
    let startup_error = match blueprint
        .create_new_rollup_with_genesis_source(
            GenesisSource::CustomParams(genesis_params),
            rollup_config.clone(),
            RollupProverConfig::Disabled,
            None,
            None,
            None,
            false,
        )
        .await
    {
        Ok(_) => panic!("rollup startup unexpectedly succeeded"),
        Err(error) => error,
    };

    assert!(
        startup_error
            .to_string()
            .contains("Missing `[proof_manager]` section"),
        "{startup_error:#}"
    );

    let mut storage_manager = blueprint
        .create_storage_manager(&rollup_config, false)
        .unwrap();
    let genesis_header = MockBlock::default().header;
    let (_, ledger_state) = storage_manager.create_state_after(&genesis_header).unwrap();
    let ledger_db = LedgerDb::with_reader(ledger_state).unwrap();
    assert!(ledger_db.get_head_slot().unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Fails too often on my machine"]
async fn flaky_test_rollup_shutdown_works_as_expected() {
    let _guard = initialize_logging();

    let dir = Arc::new(tempfile::tempdir().unwrap());

    start_and_stop_node_in_dir(dir.clone()).await;
    start_and_stop_node_in_dir(dir.clone()).await;

    Arc::into_inner(dir)
        .expect("Someone is still holding on to the directory, but everything was shutdown.")
        .close()
        .expect("Node storage directory didn't close successfully.");
}

async fn start_and_stop_node_in_dir(dir: Arc<TempDir>) {
    let genesis_params = GenesisParams {
        runtime: <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            HighLevelOptimisticGenesisConfig::generate().into(),
        ),
    };

    let test_rollup = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        1,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir);
        c.rollup_prover_config = RollupProverConfig::Disabled;
    })
    .with_standard_sequencer()
    .start()
    .await
    .unwrap();

    test_rollup.shutdown().await.unwrap();
}

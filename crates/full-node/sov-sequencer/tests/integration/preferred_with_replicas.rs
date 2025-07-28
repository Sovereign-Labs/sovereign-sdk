use std::sync::Arc;
use std::time::Duration;

use sov_mock_da::BlockProducingConfig;
use sov_modules_api::Runtime;
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::PaymasterConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::{
    TestSpec, TestUser, TEST_BLOB_PROCESSING_TIMEOUT, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE,
};
use sov_value_setter::ValueSetterConfig;

#[allow(unused_imports)]
use crate::preferred_end_to_end::{
    run_action_against_test_rollup, run_actions_against_test_rollup,
    setup_test_rollup_with_initial_state, InvalidGeneration, TestBlueprint, TestRuntime, TestState,
    TestingAction,
};
use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, MAX_BATCH_EXECUTION_TIME_MILLIS};

async fn create_test_rollups(
    num_replicas: u64,
) -> (
    Option<Vec<TestRollup<TestBlueprint>>>,
    Arc<tempfile::TempDir>,
    TestUser<TestSpec>,
) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
            (),
            PaymasterConfig::default(),
            (),
        );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = tempdir_inside_codebase_dir();

    (
        new_test_rollup::<TestRuntime<TestSpec>>(
            dir.clone(),
            genesis_params
                .runtime
                .sequencer_registry
                .sequencer_config
                .seq_da_address,
            genesis_params,
            0,
            true,
            TEST_MAX_BATCH_SIZE,
            BlockProducingConfig::Manual,
            None,
            TEST_BLOB_PROCESSING_TIMEOUT,
            num_replicas,
            MAX_BATCH_EXECUTION_TIME_MILLIS,
            None,
            TEST_FINALIZATION_BLOCKS,
        )
        .await,
        dir,
        admin,
    )
}

type MasterAndReplicasAndState = (
    TestRollup<TestBlueprint>,
    Vec<Option<TestRollup<TestBlueprint>>>,
    TestState,
);
async fn test_actions_against_replicas(
    admin: &TestUser<TestSpec>,
    (master, mut replicas, state): MasterAndReplicasAndState,
    actions: Vec<TestingAction>,
) -> MasterAndReplicasAndState {
    let (master, mut state) =
        run_actions_against_test_rollup(actions, master, &admin.clone(), state).await;

    // Ensure replicas have processed the database changes
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Verify state synchronization across all replicas
    for replica_opt in &mut replicas {
        let replica = replica_opt.take().unwrap();
        let updated_replica = run_action_against_test_rollup(
            replica,
            &admin.private_key,
            TestingAction::QuerySetValue,
            &mut state,
        )
        .await
        .unwrap();
        *replica_opt = Some(updated_replica);
    }
    (master, replicas, state)
}
async fn restart_replica(
    admin: &TestUser<TestSpec>,
    mut replicas: Vec<Option<TestRollup<TestBlueprint>>>,
    test_state: &mut TestState,
    index: usize,
) -> Vec<Option<TestRollup<TestBlueprint>>> {
    let replica = replicas[index].take().unwrap();
    let replica = run_action_against_test_rollup(
        replica,
        admin.private_key(),
        TestingAction::Restart,
        test_state,
    )
    .await
    .unwrap();
    replicas[index] = Some(replica);
    replicas
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_with_replicas() {
    let (test_rollups, _tempdir, admin) = create_test_rollups(4).await;
    let Some(test_rollups) = test_rollups else {
        return;
    };
    let mut test_rollups = test_rollups.into_iter();

    let master = test_rollups.next().unwrap();
    let replicas: Vec<Option<_>> = test_rollups.map(Some).collect();

    let (master, state) = setup_test_rollup_with_initial_state(master, &admin).await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let actions = vec![TestingAction::AcceptTx, TestingAction::QuerySetValue];
    let (master, replicas, mut state) =
        test_actions_against_replicas(&admin, (master, replicas, state), actions).await;
    let replicas = restart_replica(&admin, replicas, &mut state, 0).await;

    let actions = vec![
        TestingAction::NewDaSlot,
        TestingAction::Sleep { duration_ms: 100 },
    ];
    let (master, replicas, mut state) =
        test_actions_against_replicas(&admin, (master, replicas, state), actions).await;

    let replicas = restart_replica(&admin, replicas, &mut state, 0).await;

    let actions = vec![
        TestingAction::AcceptTxs { count: 10 },
        TestingAction::NewDaSlot,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::DuplicateTransaction,
        },
        TestingAction::Sleep { duration_ms: 100 },
        TestingAction::NewDaSlot,
        TestingAction::Sleep { duration_ms: 100 },
        TestingAction::NewDaSlot,
        TestingAction::Sleep { duration_ms: 100 },
    ];
    let (master, replicas, mut state) =
        test_actions_against_replicas(&admin, (master, replicas, state), actions).await;

    let replicas = restart_replica(&admin, replicas, &mut state, 1).await;
    let replicas = restart_replica(&admin, replicas, &mut state, 2).await;

    let (master, replicas, state) = test_actions_against_replicas(
        &admin,
        (master, replicas, state),
        vec![TestingAction::QuerySetValue],
    )
    .await;

    // Silence unused variable warnings to keep the test easier to edit
    drop(state);

    // Shutdown replicas first
    for replica in replicas {
        replica.unwrap().shutdown().await.unwrap();
    }
    // Shut down master last, otherwise the postgres subscription will drop and replicas will error
    master.shutdown().await.unwrap();
}

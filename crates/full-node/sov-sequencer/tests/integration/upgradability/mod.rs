use crate::utils::{
    new_test_rollup, pause_update_state, tx_set_value_with_gas, MAX_BATCH_EXECUTION_TIME_MILLIS,
};
use futures::StreamExt;
use sov_api_spec::{types, ResponseValue};
use sov_kernels::soft_confirmations::SoftConfirmationsKernel;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{RawTx, Runtime};
use sov_node_client::NodeClient;
use sov_test_utils::runtime::genesis::operator::HighLevelOperatorGenesisConfig;
use sov_test_utils::runtime::GenesisParams;
use sov_test_utils::test_rollup::get_height;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::{
    generate_operator_runtime_with_kernel, RtAgnosticBlueprint, TestSpec, TestUser,
    TEST_BLOB_PROCESSING_TIMEOUT, TEST_DEFAULT_USER_BALANCE, TEST_FINALIZATION_BLOCKS,
    TEST_MAX_BATCH_SIZE, TEST_NORMAL_SHUTDOWN_TIMEOUT,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use std::sync::Arc;
use std::time::Duration;

generate_operator_runtime_with_kernel!(kernel_type: SoftConfirmationsKernel<'a, S>, TestRuntime <= value_setter: ValueSetter<S>);
type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

#[tokio::test(flavor = "multi_thread")]
async fn flaky_tests_sequencer_stops_if_stop_at_height_too_small_immediate_finality() {
    sequencer_stops_if_stop_at_height_too_small(0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_tests_sequencer_stops_if_stop_at_height_too_small() {
    sequencer_stops_if_stop_at_height_too_small(TEST_FINALIZATION_BLOCKS).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_tests_sequencer_does_not_accept_tx_after_stop_immediate_finality() {
    sequencer_does_not_accept_tx_after_stop(0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_tests_sequencer_does_not_accept_tx_after_stop() {
    sequencer_does_not_accept_tx_after_stop(TEST_FINALIZATION_BLOCKS).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_rollup_operates_only_on_finalized_blocks_if_stop_at_immediate_finality() {
    rollup_operates_only_on_finalized_blocks_if_stop_at_height_set(0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_rollup_operates_only_on_finalized_blocks_if_stop_at_height_set() {
    rollup_operates_only_on_finalized_blocks_if_stop_at_height_set(TEST_FINALIZATION_BLOCKS).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_start_at_immediate_finality() {
    check_start_at(0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_start_at_finalization_minus_one() {
    check_start_at(TEST_FINALIZATION_BLOCKS - 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_start_at_finalization_threshold() {
    check_start_at(TEST_FINALIZATION_BLOCKS).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_start_at_finalization_plus_one() {
    check_start_at(TEST_FINALIZATION_BLOCKS + 1).await;
}

async fn sequencer_stops_if_stop_at_height_too_small(finalization_blocks: u32) {
    let stop_at_height = RollupHeight::new(3);

    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        None,
        finalization_blocks,
    )
    .await;

    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();
    let api_client = test_rollup.api_client().clone();

    // Produce the minimum paced DA blocks needed for readiness.
    // We intentionally avoid `produce_enough_finalized_slots()` here because it also performs
    // extra sync/lag advancement work, which can move this test into transient
    // "node not synced yet" states and make pre-stop tx checks flaky.
    // `+2` gives a deterministic post-genesis finalized update observed by the poller.
    test_rollup
        .tenderly_produce_blocks((finalization_blocks + 2) as usize)
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Produce enough blocks with transactions to advance rollup height past stop_at_height.
    // Transactions are required to trigger batch production and increment rollup height.
    let target_height = stop_at_height.get() + finalization_blocks as u64 + 2;
    let mut nonce = 0;
    while test_rollup.height().await.get() < target_height {
        send_tx(&admin, nonce, &api_client).await.unwrap();
        nonce += 1;
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
    }

    let slot_height = test_rollup.height().await;

    // Assert the condition that triggers an early return.
    assert!(stop_at_height < slot_height);

    // Ensure that state up to at least `stop_at_height` is finalized and persisted to disk.
    // Without this, non-finalized blocks' state changes are lost on shutdown, causing the
    // restart to see a lower height than expected.
    test_rollup.produce_enough_finalized_slots().await;

    let Err(err) = test_rollup
        .restart_with_heights(None, Some(stop_at_height))
        .await
    else {
        panic!("The rollup should have stopped")
    };

    assert!(err.to_string().contains("The requested stop_height"));
}

async fn sequencer_does_not_accept_tx_after_stop(finalization_blocks: u32) {
    let shutdown_timeout = Duration::from_secs(10);

    let stop_at_height = RollupHeight::new((finalization_blocks + 12) as u64);

    let (mut test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        Some(stop_at_height),
        finalization_blocks,
    )
    .await;

    let expected_error = format!(
        "The preferred sequencer has reached the stop height {} and is no longer accepting transactions.",
        stop_at_height.get()
    );

    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();

    // Produce the minimum paced DA blocks needed for readiness.
    // We intentionally avoid `produce_enough_finalized_slots()` here because it also performs
    // extra sync/lag advancement work, which can move this test into transient
    // "node not synced yet" states and make pre-stop tx checks flaky.
    // `+2` gives a deterministic post-genesis finalized update observed by the poller.
    test_rollup
        .tenderly_produce_blocks((finalization_blocks + 2) as usize)
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let api_client = test_rollup.api_client().clone();

    let mut nonce = 0;
    let mut current_height = test_rollup.height().await;

    while current_height.get() < stop_at_height.get() {
        // Height check and tx submission are not atomic. If the node reaches stop-height
        // between the check and the submission, rejection is expected and we should exit.
        match send_tx(&admin, nonce, &api_client).await {
            Ok(_) => {
                nonce += 1;
            }
            Err(err) => {
                assert!(
                    err.contains(&expected_error),
                    "Unexpected pre-stop tx error: {err}"
                );
                break;
            }
        }

        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
        current_height = test_rollup.height().await;
    }

    test_rollup.wait_for_height(stop_at_height.get()).await;

    // After the stop height is reached, the sequencer should not accept any transactions. Until the height is finalized.
    for _ in 0..finalization_blocks {
        let Ok(current_height) = get_height(&test_rollup.client).await else {
            // Shutdown may race with this verification loop in CI.
            break;
        };
        assert_eq!(current_height, stop_at_height);

        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;

        let err = send_tx(&admin, nonce, &api_client).await.unwrap_err();
        nonce += 1;
        assert!(err.contains(&expected_error));
    }

    for _ in 0..3 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
    }

    // We use "standard" MockDa block time to not overwhelm node.
    let shutdown_wait_step =
        Duration::from_millis(sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS);
    let shutdown_deadline = tokio::time::Instant::now() + shutdown_timeout;

    loop {
        match test_rollup
            .try_wait_for_rollup_to_shutdown(shutdown_wait_step)
            .await
        {
            Ok(()) => break,
            Err(error) => {
                let is_wait_timeout = error
                    .to_string()
                    .contains("Failed to join rollup task before timeout");
                if !is_wait_timeout {
                    panic!("Rollup shutdown failed unexpectedly: {error:#}");
                }
                if tokio::time::Instant::now() >= shutdown_deadline {
                    panic!(
                        "Failed waiting for rollup shutdown before timeout (stop_at_height={stop_at_height}): {error:#}"
                    );
                }
                test_rollup.da_service.produce_block_now().await.unwrap();
            }
        }
    }
}

async fn rollup_operates_only_on_finalized_blocks_if_stop_at_height_set(finalization_blocks: u32) {
    assert!(finalization_blocks < 10);
    let stop_at_height = RollupHeight::new(15);

    let (test_rollup, _) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        Some(stop_at_height),
        finalization_blocks,
    )
    .await;

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    // This is for make rollup operational, so rollup will give out slot notifications.
    test_rollup
        .tenderly_produce_blocks((finalization_blocks + 1) as usize)
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.client.clone();
    let mut current_height = get_height(&client).await.unwrap();
    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();
    // Start producing blocks:
    // and wait till rollup reaches "stop height".
    // make sure sequencer stops producing batches and rollup only processes finalized headers.
    // Height in this loop is from state of the sequencer
    while current_height < stop_at_height {
        // Each DA block triggers slot notification, even if it does not have any blobs.
        test_rollup.da_service.produce_block_now().await.unwrap();
        let _slot = slot_subscription.next().await.unwrap().unwrap();
        assert_rollup_processes_only_finalized_blocks(&client).await;
        current_height = test_rollup.height().await;
    }

    // At this point sequencer is not producing batches anymore, as it reached stop rollup height
    // Now we need to make sure, that runner finalizes this height.

    // We need to create `finalization + 1` blocks to make sure
    for _ in 0..=(finalization_blocks + 1) {
        let Ok(current_height) = get_height(&client).await else {
            // Rollup might've shut down already and the request is going to fail.
            break;
        };
        assert_eq!(current_height, stop_at_height);
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
    }

    test_rollup
        .wait_for_rollup_to_shutdown(TEST_NORMAL_SHUTDOWN_TIMEOUT)
        .await;
}

async fn check_start_at(finalization_blocks: u32) {
    let stop_at_height = RollupHeight::new(15);

    let (test_rollup, _) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        Some(stop_at_height),
        finalization_blocks,
    )
    .await;

    let client = test_rollup.client.clone();

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    test_rollup
        .tenderly_produce_blocks((finalization_blocks + 1) as usize)
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut shutdown_rec = test_rollup.shutdown_sender.subscribe();
    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();

    let mut last_height = RollupHeight::new(0);
    tokio::time::timeout(Duration::from_secs(30), async {
        // Wait until the rollup reaches `stop_at_height`.
        // At that point, we shut down and `get_height` is expected to return errors.
        // The slot waiting is used to prevent a busy loop, not for correctness.
        while let Ok(height) = get_height(&client).await {
            test_rollup.da_service.produce_block_now().await.unwrap();
            slot_subscription.next().await;
            last_height = height;
        }
    })
    .await
    .unwrap();

    assert_eq!(last_height, stop_at_height);

    // Let's wait for the shutdown.
    shutdown_rec.changed().await.unwrap();

    pause_update_state::set(true);

    // The correct starting height should be `stop_at_height + 1`.
    let start_at = stop_at_height.checked_add(1).unwrap();
    let test_rollup = test_rollup
        .restart_with_heights(Some(start_at), None)
        .await
        .unwrap();

    let client = test_rollup.client.clone();
    let current_height = get_height(&client).await.unwrap();

    pause_update_state::set(false);

    // Verify that the last processed height was `stop_at_height`. Since we called `pause_update_state`,
    // we should see the final height before the shutdown.
    assert_eq!(current_height, stop_at_height);
    test_rollup.shutdown().await.unwrap();
}

async fn assert_rollup_processes_only_finalized_blocks(client: &NodeClient) {
    let last_finalized_slot_number = get_last_finalized_slot_number(client).await;
    let last_slot_number = get_last_slot_number(client).await;
    // During the upgrade procedure, rollup processes only finalized blocks.
    assert_eq!(
        last_finalized_slot_number,
        last_slot_number,
        "left is last finalized slot number {last_finalized_slot_number}, right is last slot number: {last_slot_number}");
}

async fn send_tx(
    admin: &TestUser<TestSpec>,
    nonce: u64,
    api_client: &sov_api_spec::Client,
) -> Result<ResponseValue<types::TxInfoWithConfirmation>, String> {
    let tx = tx_set_value(&admin.private_key, nonce, 8);
    let res = api_client.send_raw_tx_to_sequencer(&tx).await;

    match res {
        Ok(ok) => Ok(ok),
        Err(sov_api_spec::Error::ErrorResponse(err)) => Err(err.into_inner().message),
        Err(err) => {
            panic!("Unexpected error: {err:?}")
        }
    }
}

async fn get_last_finalized_slot_number(client: &NodeClient) -> u64 {
    get_slot_number(client, true).await
}

async fn get_last_slot_number(client: &NodeClient) -> u64 {
    get_slot_number(client, false).await
}

async fn get_slot_number(client: &NodeClient, finalized: bool) -> u64 {
    let url = if finalized {
        "/ledger/slots/finalized"
    } else {
        "/ledger/slots/latest"
    };
    let response = client.http_get(url).await.unwrap();
    let slot: types::Slot = serde_json::from_str(&response).unwrap();
    slot.number
}

#[allow(clippy::too_many_arguments)]
async fn create_test_rollup(
    minimum_profit_per_tx: u128,
    max_batch_size: usize,
    blob_processing_timeout_secs: u64,
    stop_at_rollup_height: Option<RollupHeight>,
    finalization_blocks: u32,
) -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let reward_user = TestUser::<TestSpec>::generate(TEST_DEFAULT_USER_BALANCE);

    let genesis_config =
        HighLevelOperatorGenesisConfig::<TestSpec>::generate_with_additional_accounts(
            2,
            reward_user,
        );

    let admin = genesis_config.additional_accounts()[0].clone();
    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
        );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = Arc::new(tempfile::tempdir().unwrap());

    (
        new_test_rollup::<TestRuntime<TestSpec>>(
            dir.clone(),
            genesis_params
                .runtime
                .sequencer_registry
                .sequencer_config
                .seq_da_address,
            genesis_params,
            minimum_profit_per_tx,
            true,
            max_batch_size,
            BlockProducingConfig::Manual,
            None,
            blob_processing_timeout_secs,
            MAX_BATCH_EXECUTION_TIME_MILLIS,
            stop_at_rollup_height,
            finalization_blocks,
        )
        .await,
        admin,
    )
}

fn tx_set_value(key: &Ed25519PrivateKey, nonce: u64, value_to_set: u64) -> RawTx {
    tx_set_value_with_gas::<TestRuntime<TestSpec>>(
        key,
        nonce,
        value_to_set,
        None,
        sov_test_utils::TEST_DEFAULT_MAX_FEE,
    )
}

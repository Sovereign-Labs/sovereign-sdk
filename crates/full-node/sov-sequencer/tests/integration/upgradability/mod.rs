use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use sov_api_spec::{types, ResponseValue};
use sov_kernels::soft_confirmations::SoftConfirmationsKernel;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{RawTx, Runtime};
use sov_node_client::NodeClient;
use sov_test_utils::logging::LogCollector;
use sov_test_utils::runtime::genesis::operator::HighLevelOperatorGenesisConfig;
use sov_test_utils::runtime::GenesisParams;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::{
    generate_operator_runtime_with_kernel, RtAgnosticBlueprint, TestSpec, TestUser,
    TEST_BLOB_PROCESSING_TIMEOUT, TEST_DEFAULT_USER_BALANCE, TEST_FINALIZATION_BLOCKS,
    TEST_MAX_BATCH_SIZE,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry;

use crate::utils::{
    new_test_rollup, pause_update_state, tx_set_value_with_gas, MAX_BATCH_EXECUTION_TIME_MILLIS,
};

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
async fn test_start_at() {
    check_start_at(TEST_FINALIZATION_BLOCKS - 1).await;
    check_start_at(TEST_FINALIZATION_BLOCKS).await;
    check_start_at(TEST_FINALIZATION_BLOCKS + 1).await;
}

async fn sequencer_stops_if_stop_at_height_too_small(finalization_blocks: u32) {
    let collector = LogCollector::new(Level::ERROR);
    let subscriber = registry().with(collector.clone());
    subscriber.init();

    let stop_at_height = RollupHeight::new(3);

    let (test_rollup, _) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        None,
        finalization_blocks,
    )
    .await;

    let test_rollup = test_rollup.unwrap();

    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();

    for _ in 0..20 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let client = test_rollup.client.clone();
    let slot_height = get_height(&client).await.unwrap();

    // Assert the condition that triggers an early return.
    assert!(stop_at_height < slot_height);

    let Err(err) = test_rollup
        .restart_with_heights(None, Some(stop_at_height))
        .await
    else {
        panic!("The rollup should have stopped")
    };

    let mut records = collector.records();
    let (_, log) = records.remove(0);

    assert!(log.contains("The requested stop_height "));
    assert!(err.to_string().contains("The requested stop_height"));
}

async fn sequencer_does_not_accept_tx_after_stop(finalization_blocks: u32) {
    let stop_at_height = RollupHeight::new((finalization_blocks + 12) as u64);

    let (test_rollup, admin) = create_test_rollup(
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

    let test_rollup = test_rollup.unwrap();
    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();

    let client = test_rollup.client.clone();

    let da = test_rollup.da_service.clone();
    let mut da_sub = da.subscribe_finalized_header().await.unwrap();
    // We just need at least one finalized block to proceed with the tests.
    for _ in 0..finalization_blocks + 3 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    da_sub.next().await;

    let api_client = test_rollup.api_client().clone();

    let mut nonce = 0;
    let mut current_height = get_height(&client).await.unwrap();

    while current_height.get() < stop_at_height.get() {
        // All transactions should be accepted until the stop height is reached.
        send_tx(&admin, nonce, &api_client).await.unwrap();
        nonce += 1;

        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;

        tokio::time::sleep(Duration::from_millis(300)).await;
        current_height = get_height(&client).await.unwrap();
    }

    // After the stop height is reached, the sequencer should not accept any transactions. Until the height is finalized.
    for _ in 0..finalization_blocks {
        let current_height = get_height(&client).await.unwrap();
        assert_eq!(current_height, stop_at_height);

        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;

        tokio::time::sleep(Duration::from_millis(300)).await;
        let err = send_tx(&admin, nonce, &api_client).await.unwrap_err();
        nonce += 1;
        assert!(err.contains(&expected_error));
    }

    for _ in 0..3 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(1))
        .await;
}

async fn rollup_operates_only_on_finalized_blocks_if_stop_at_height_set(finalization_blocks: u32) {
    let stop_at_height = RollupHeight::new(15);

    let (test_rollup, _) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        Some(stop_at_height),
        finalization_blocks,
    )
    .await;

    let test_rollup = test_rollup.unwrap();

    let client = test_rollup.client.clone();

    test_rollup
        .da_service
        .produce_n_blocks_now(10)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut current_height = get_height(&client).await.unwrap();
    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();
    while current_height < stop_at_height {
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
        // We need to wait a bit so the new block is visible to the sequencer.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_rollup_processes_only_finalized_blocks(&client).await;
        current_height = get_height(&client).await.unwrap();
    }

    for _ in 0..finalization_blocks + 1 {
        current_height = get_height(&client).await.unwrap();
        assert_eq!(current_height, stop_at_height);
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(3))
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

    let test_rollup = test_rollup.unwrap();
    let client = test_rollup.client.clone();

    test_rollup
        .da_service
        .produce_n_blocks_now(10)
        .await
        .unwrap();

    let mut shutdown_rec = test_rollup.shutdown_sender.subscribe();
    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();

    let mut last_height = RollupHeight::new(0);
    tokio::time::timeout(Duration::from_secs(25), async {
        // Wait until the rollup reaches `stop_at_height`. At that point, we shutdown and `get_height` is expected to return errors.
        // The sleep is used to prevent a busy loop, not for correctness.
        while let Ok(height) = get_height(&client).await {
            test_rollup.da_service.produce_block_now().await.unwrap();
            slot_subscription.next().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
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
    let last_finalized_block_height = get_last_finalized_block_height(client).await;
    let last_block_height = get_last_block_height(client).await;
    // During the upgrade procedure rollup processes only finalized blocks.
    assert_eq!(last_finalized_block_height, last_block_height);
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

use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct Data {
    value: (u64, u64),
}

async fn get_height(client: &NodeClient) -> anyhow::Result<RollupHeight> {
    let url = "/modules/chain-state/state/current-heights";
    let response = client.http_get(url).await?;
    let heights: Data = serde_json::from_str(&response)?;
    Ok(RollupHeight::new(heights.value.0))
}

async fn get_last_finalized_block_height(client: &NodeClient) -> u64 {
    get_block_height(client, true).await
}

async fn get_last_block_height(client: &NodeClient) -> u64 {
    get_block_height(client, false).await
}

async fn get_block_height(client: &NodeClient, finalized: bool) -> u64 {
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
) -> (Option<TestRollup<TestBlueprint>>, TestUser<TestSpec>) {
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
            1,
            MAX_BATCH_EXECUTION_TIME_MILLIS,
            stop_at_rollup_height,
            finalization_blocks,
        )
        .await
        .map(|v| v.into_iter().next().unwrap()),
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

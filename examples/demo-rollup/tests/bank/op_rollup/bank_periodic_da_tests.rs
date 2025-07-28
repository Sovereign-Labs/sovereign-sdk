use core::time::Duration;
use std::sync::Arc;
use std::thread::sleep;

use anyhow::Context;
use futures::StreamExt;
use serde::Deserialize;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::OperatingMode;
use sov_test_utils::test_rollup::RollupBuilder;

use crate::bank::helpers::*;
use crate::bank::{TOKEN_DECIMALS, TOKEN_NAME};
use crate::test_helpers::{test_genesis_source, DemoRollupSpec};

const BLOCK_TIME_MS: u64 = 100;
const ESTIMATED_BLOCK_PROCESSING_TIME: Duration = Duration::from_millis(BLOCK_TIME_MS);
const BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: BLOCK_TIME_MS,
};

#[tokio::test(flavor = "multi_thread")]
async fn bank_tx_periodic_da_tests() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");

    let test_case = TestCase {
        wait_for_aggregated_proof: true,
        finalization_blocks: 0,
    };

    let test_rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Optimistic),
        BLOCK_PRODUCING_CONFIG,
        test_case.finalization_blocks,
    )
    .set_config(|c| c.max_concurrent_blobs = 16777216)
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .start()
    .await?;

    test_rollup
        .da_service
        .produce_n_blocks_now(3)
        .await
        .unwrap();

    // If the rollup throws an error, return it and stop trying to send the transaction
    tokio::select! {
        err = test_rollup.rollup_task => err?,
        res = send_test_bank_txs(test_case, &test_rollup.client, test_rollup.da_service.clone()) => Ok(res?),
    }?;

    Ok(())
}

async fn send_test_bank_txs(
    test_case: TestCase,
    client: &NodeClient,
    da_service: Arc<StorableMockDaService>,
) -> anyhow::Result<()> {
    let mut slots_subscription = client.client.subscribe_slots().await?;

    let (key, user_address, token_id, _recipient_address) = create_keys_and_addresses();
    let token_id_response = client
        .get_token_id::<DemoRollupSpec>(TOKEN_NAME, Some(TOKEN_DECIMALS), &user_address)
        .await?;
    assert_eq!(token_id, token_id_response);

    // create token. height 1
    let initial_balance = 1000;
    let tx = build_create_token_tx(&key, 0, initial_balance);

    da_service.produce_n_blocks_now(3).await.unwrap();

    let batch_1_rollup_height = send_tx_and_wait_for_status(&[tx], client).await?;

    assert!(batch_1_rollup_height >= 1);

    // FIXME(@theochap): Remove that once we are confident that we don't have a race condition in the sequencer.
    sleep(Duration::from_millis(
        (ESTIMATED_BLOCK_PROCESSING_TIME.as_millis() * config_value!("DEFERRED_SLOTS_COUNT") * 2)
            .try_into()
            .unwrap(),
    ));

    assert_slot_finality(
        client,
        batch_1_rollup_height,
        test_case.expected_head_finality(),
    )
    .await;

    assert_balance(client, initial_balance, token_id, user_address, None)
        .await
        .context("Initial balance after token create")?;

    // Since we don't have control on which DA blocks attestation lends,
    // We check that all slots up to slot with transactions has been attested.

    let mut rollup_height = 1;
    let mut verified_attested_height = 0;

    // How many slots rollup allowed to lag behind in posting attestations
    let attestation_publish_threshold = 1000;

    while verified_attested_height <= batch_1_rollup_height {
        let slot = slots_subscription.next().await.unwrap()?;
        assert!(slot.number >= rollup_height);

        let max_attested_height = get_max_attested_height(client, Some(rollup_height)).await?;
        if max_attested_height >= verified_attested_height {
            // We can have several attestations in the same DA block, so we need to set `verified_attested_height` to the `max_attested_height`.
            verified_attested_height = max_attested_height;
        }
        rollup_height += 1;
        if rollup_height > (batch_1_rollup_height + attestation_publish_threshold) {
            panic!(
                "Attestations haven't been posted after {attestation_publish_threshold} slots passed since batch publication"
            );
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct ValueResponse {
    value: u64,
}

async fn get_max_attested_height(
    client: &NodeClient,
    rollup_height: Option<u64>,
) -> anyhow::Result<u64> {
    let param = rollup_height
        .map(|h| format!("?rollup_height={h}"))
        .unwrap_or_default();
    let url = format!("/modules/attester-incentives/state/maximum-attested-height{param}");
    let response = client.query_rest_endpoint::<ValueResponse>(&url).await?;

    let height = response.value;
    Ok(height)
}

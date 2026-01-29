#![allow(dead_code, unused_imports, unused_variables)]
use crate::bank::helpers::*;
use crate::bank::{TOKEN_DECIMALS, TOKEN_NAME};
use crate::test_helpers::*;
use anyhow::Context;
use futures::StreamExt;
use serde::Deserialize;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_full_node_configs::sequencer::{RecoveryStrategy, SequencerKindConfig};
use sov_mock_da::storable::StorableMockDaService;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::OperatingMode;
use sov_test_utils::test_rollup::{RollupBuilder, RollupProverConfig};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "fix when ZKP work is resumed again"]
async fn bank_tx_tests() -> anyhow::Result<()> {
    sov_test_utils::logging::initialize_or_change_logging_with_filter("warn");
    let test_case = TestCase {
        wait_for_aggregated_proof: true,
        finalization_blocks: 0,
    };

    let test_rollup = start_test_rollup(&test_case, OperatingMode::Optimistic).await?;

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
    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();
    let token_id_response = client
        .get_token_id::<DemoRollupSpec>(TOKEN_NAME, Some(TOKEN_DECIMALS), &user_address)
        .await?;

    const NUM_TRANSFERS: u64 = 5;

    assert_eq!(token_id, token_id_response);

    // create token.
    let initial_balance = 1000;
    let tx = build_create_token_tx(&key, 0, initial_balance);

    let mut slots_subscription = client.client.subscribe_slots().await?;
    let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;

    let mut processed_slot = slots_subscription.next().await.unwrap()?;
    while processed_slot.number < slot_number {
        processed_slot = slots_subscription.next().await.unwrap()?;
    }

    assert_slot_finality(client, slot_number, test_case.expected_head_finality()).await;
    assert_balance(client, initial_balance, token_id, user_address, None).await?;

    // Make a few transfers and check that attestation height progresses
    for nonce in 1..=NUM_TRANSFERS {
        let tx = build_transfer_token_tx(&key, token_id, recipient_address, 10, nonce);

        let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;

        assert_slot_finality(client, slot_number, test_case.expected_head_finality()).await;
        assert_balance(
            client,
            initial_balance - (nonce as u128) * 10,
            token_id,
            user_address,
            None,
        )
        .await?;

        tracing::info!(slot_number, "Tx has been published in slot");
        while processed_slot.number < slot_number {
            processed_slot = slots_subscription.next().await.unwrap()?;
        }
        tracing::info!("C {nonce} =========");
        tracing::info!("------------------------");

        let mut max_attested_height = get_max_attested_height(client).await?;
        tracing::info!(max_attested_height, "Starting max attested height");
        while max_attested_height < slot_number {
            // In some cases, the attestation may not be ready just yet. Let's try again in a bit.
            max_attested_height = get_max_attested_height(client)
                .await
                .with_context(|| format!("Final part of the test: failed to get max attested height for {slot_number}, only have {max_attested_height}"))?;
            tracing::info!(
                "KEEP WAITING: {nonce} =========: {max_attested_height} < {slot_number}"
            );
            slots_subscription.next().await.unwrap()?;
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct ValueResponse {
    value: u64,
}

async fn get_max_attested_height(client: &NodeClient) -> anyhow::Result<u64> {
    let response = client
        .query_rest_endpoint::<ValueResponse>(
            "/modules/attester-incentives/state/maximum-attested-height",
        )
        .await?;

    let height = response.value;
    Ok(height)
}

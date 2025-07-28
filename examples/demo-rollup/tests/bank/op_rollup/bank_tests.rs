#![allow(dead_code, unused_imports, unused_variables)]
use std::sync::Arc;

use serde::Deserialize;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_da::storable::service::StorableMockDaService;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::OperatingMode;
use sov_test_utils::test_rollup::{RollupBuilder, RollupProverConfig};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::bank::helpers::*;
use crate::bank::{TOKEN_DECIMALS, TOKEN_NAME};
use crate::test_helpers::*;

#[tokio::test(flavor = "multi_thread")]
async fn flaky_bank_tx_tests() -> anyhow::Result<()> {
    // std::env::set_var("RUST_LOG", "info");
    // sov_test_utils::initialize_logging();
    let test_case = TestCase {
        wait_for_aggregated_proof: true,
        finalization_blocks: 0,
    };

    let test_rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Optimistic),
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        test_case.finalization_blocks,
    )
    .set_config(|c| c.max_concurrent_blobs = 65536)
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .start()
    .await?;

    // We need a handful of blocks for the sequencer to be able to advance the
    // visible slot number. Fewer blocks could possibly be enough as well, I
    // haven't counted (@neysofu).
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
    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();
    let token_id_response = client
        .get_token_id::<DemoRollupSpec>(TOKEN_NAME, Some(TOKEN_DECIMALS), &user_address)
        .await?;

    const NUM_TRANSFERS: u64 = 5;

    assert_eq!(token_id, token_id_response);

    // create token.
    let initial_balance = 1000;
    let tx = build_create_token_tx(&key, 0, initial_balance);

    let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;

    // Will cause a batch to be produced.
    da_service.produce_n_blocks_now(1).await.unwrap();

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

        let mut max_attested_height = get_max_attested_height(client).await?;
        while max_attested_height < slot_number {
            // In some cases, the attestation may not be ready just yet. Let's try again in a bit.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            max_attested_height = get_max_attested_height(client).await?;
        }

        da_service.produce_n_blocks_now(1).await.unwrap();
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

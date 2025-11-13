use futures::StreamExt;
use sov_bank::config_gas_token_id;
use sov_bank::derived_holder::DerivedHolder;
use sov_cli::NodeClient;
use sov_mock_da::storable::StorableMockDaService;
use sov_modules_api::{Amount, OperatingMode, Spec};
use sov_test_utils::TestSpec;
use std::str::FromStr;
use std::sync::Arc;

use crate::bank::helpers::*;
use crate::bank::{TOKEN_DECIMALS, TOKEN_NAME};
use crate::test_helpers::*;

#[tokio::test(flavor = "multi_thread")]
async fn flaky_bank_tx_tests_secured_by_operator() -> anyhow::Result<()> {
    let test_case = TestCase {
        wait_for_aggregated_proof: false,
        finalization_blocks: 0,
    };

    let test_rollup = start_test_rollup(&test_case, OperatingMode::Operator).await?;

    // If the rollup throws an error, return it and stop trying to send the transaction
    tokio::select! {
        err = test_rollup.rollup_task => err?,
        res = send_test_bank_txs(test_case, &test_rollup.client, test_rollup.da_service.clone()) => Ok(res?),
    }?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn bank_events_test() -> anyhow::Result<()> {
    let test_case = TestCase {
        wait_for_aggregated_proof: false,
        finalization_blocks: 0,
    };

    let test_rollup = start_test_rollup(&test_case, OperatingMode::Operator).await?;

    test_rollup.wait_for_sequencer_ready().await.unwrap();
    let (key, _, token_id, recipient_address) = create_keys_and_addresses();
    let nb_of_txs = 3;

    let mut event_subscription = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/TokenTransferred")
        .await
        .unwrap();

    let initial_balance = 1000;
    let tx = build_create_token_tx(&key, 0, initial_balance);

    send_tx_and_wait_for_status(&[tx], &test_rollup.client).await?;

    for nonce in 1..=nb_of_txs {
        let tx = build_transfer_token_tx(&key, token_id, recipient_address, 10, nonce);
        send_tx_and_wait_for_status(&[tx], &test_rollup.client).await?;
    }

    for _ in 1..=nb_of_txs {
        // Wait for all the tx events or fail with timeout.
        tokio::time::timeout(std::time::Duration::from_secs(1), event_subscription.next())
            .await
            .unwrap();
    }

    Ok(())
}

async fn send_test_bank_txs(
    test_case: TestCase,
    client: &NodeClient,
    da_service: Arc<StorableMockDaService>,
) -> anyhow::Result<()> {
    let mut slots_subscription = client.client.subscribe_slots().await?;
    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();
    let token_id_response = client
        .get_token_id::<DemoRollupSpec>(TOKEN_NAME, Some(TOKEN_DECIMALS), &user_address)
        .await?;

    let reward_addr = <TestSpec as Spec>::Address::from_str(
        "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
    )?;

    {
        let reward_amount = client
            .get_balance::<TestSpec>(&reward_addr, &config_gas_token_id(), None)
            .await;
        assert!(reward_amount.is_err());
    }

    const NUM_TRANSFERS: u64 = 5;

    assert_eq!(token_id, token_id_response);

    // create token.
    let initial_balance = 1000;
    let tx = build_create_token_tx(&key, 0, initial_balance);

    let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;
    let mut processed_slot = slots_subscription.next().await.unwrap()?;
    while processed_slot.number < slot_number {
        processed_slot = slots_subscription.next().await.unwrap()?;
    }

    // Will cause a batch to be produced.
    da_service.produce_n_blocks_now(1).await.unwrap();

    assert_slot_finality(client, slot_number, test_case.expected_head_finality()).await;
    assert_balance(client, initial_balance, token_id, user_address, None).await?;

    // Make a few transfers and check that attestation height progresses
    for nonce in 1..=NUM_TRANSFERS {
        let tx = build_transfer_token_tx(&key, token_id, recipient_address, 10, nonce);

        let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;
        while processed_slot.number < slot_number {
            processed_slot = slots_subscription.next().await.unwrap()?;
        }

        assert_slot_finality(client, slot_number, test_case.expected_head_finality()).await;
        assert_balance(
            client,
            initial_balance - (nonce as u128) * 10,
            token_id,
            user_address,
            None,
        )
        .await?;

        da_service.produce_n_blocks_now(1).await?;
    }

    {
        let reward_amount = client
            .get_balance::<TestSpec>(&reward_addr, &config_gas_token_id(), None)
            .await?;
        assert_ne!(reward_amount, Amount::ZERO);
    }

    // Check the derive holder api
    let derived_holder_str = DerivedHolder::from([22; 32]).to_string();
    let derived_holder_balance_err = client
        .get_balance_for_holder::<TestSpec>(&derived_holder_str, &token_id)
        .await
        .unwrap_err()
        .to_string();

    assert!(derived_holder_balance_err.contains("404 Not Found"));

    Ok(())
}

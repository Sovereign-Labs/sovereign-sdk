use sov_api_spec::types;
use sov_bank::{config_gas_token_id, Bank};
use sov_modules_api::capabilities::config_chain_id;
use sov_modules_api::prelude::tokio::{self};
use sov_modules_api::transaction::{PriorityFeeBips, TxDetails};
use sov_modules_api::{Amount, Gas, GasArray, GasSpec, PrivateKey, Spec, SyncStatus, TxEffect};
use sov_rollup_apis::{PartialTransaction, SimulateExecutionContainer};
use sov_test_utils::{AsUser, EncodeCall, TransactionTestCase, TEST_DEFAULT_MAX_FEE};

use crate::{TestData, RT, S};

/// Tests that getting the latest base fee per gas returns the initial base fee per gas after genesis.
#[tokio::test(flavor = "multi_thread")]
async fn test_get_base_fee_per_gas_latest() {
    let data = TestData::setup().await;

    let response = data.client().get_latest_base_fee_per_gas().await.unwrap();
    assert_eq!(
        <<S as Spec>::Gas as Gas>::Price::try_from(
            response
                .base_fee_per_gas
                .0
                .iter()
                .map(|item| item.as_str().parse::<Amount>().unwrap())
                .collect::<Vec<_>>()
        )
        .unwrap(),
        S::initial_base_fee_per_gas()
    );
}

/// Tests that getting the latest base fee per gas gets updated after a slot is processed.
#[tokio::test(flavor = "multi_thread")]
async fn test_get_base_fee_per_gas_latest_with_updates() {
    let mut data = TestData::setup().await;

    let initial_response = data.client().get_latest_base_fee_per_gas().await.unwrap();

    let runner = &mut data.runner;
    let user = &data.user;

    for _ in 0..5 {
        runner.execute_transaction(TransactionTestCase {
            input: user.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Burn {
                coins: sov_bank::Coins {
                    amount: Amount::new(1000),
                    token_id: config_gas_token_id(),
                },
            }),
            assert: Box::new(move |result, _state| {
                assert!(
                    result.tx_receipt.is_successful(),
                    "The transaction should have succeeded"
                );
            }),
        });
    }

    let current_gas_price = runner
        .receipts()
        .last()
        .unwrap()
        .last_batch_receipt()
        .inner
        .gas_price
        .clone();

    let initial_gas_price = S::initial_base_fee_per_gas();

    assert!(
        current_gas_price.dim_is_less_than(&initial_gas_price),
        "The gas price in the runner should have decreased! Current gas price {current_gas_price}, initial gas price {initial_gas_price}"
    );

    data.send_storage();

    let response = data
        .client()
        .get_latest_base_fee_per_gas()
        .await
        .unwrap()
        .clone();

    let api_initial_gas_price = <<S as Spec>::Gas as Gas>::Price::try_from(
        initial_response
            .base_fee_per_gas
            .0
            .iter()
            .map(|item| item.as_str().parse::<Amount>().unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let api_current_gas_price = <<S as Spec>::Gas as Gas>::Price::try_from(
        response
            .base_fee_per_gas
            .0
            .iter()
            .map(|item| item.as_str().parse::<Amount>().unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap();

    // The gas price should match the initial gas price
    assert_eq!(api_initial_gas_price, initial_gas_price);

    // The gas price should decrease because the slot doesn't have enough gas
    assert!(
        api_current_gas_price.dim_is_less_than(&api_initial_gas_price),
        "The gas price should have decreased, but it didn't: current gas price {api_current_gas_price}, initial gas price {api_initial_gas_price}"
    );

    assert_eq!(
        api_current_gas_price, current_gas_price,
        "The api gas price should be the same as the current gas price"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulation() {
    let mut data = TestData::setup().await;

    let partial_tx: types::PartialTransaction = PartialTransaction::<S> {
        sender_pub_key: data.user.private_key().pub_key(),
        details: TxDetails {
            max_priority_fee_bips: PriorityFeeBips::ZERO,
            max_fee: TEST_DEFAULT_MAX_FEE,
            gas_limit: None,
            chain_id: config_chain_id(),
        },
        encoded_call_message: <RT as EncodeCall<Bank<S>>>::encode_call(
            sov_bank::CallMessage::Burn {
                coins: sov_bank::Coins {
                    amount: Amount::new(1000),
                    token_id: config_gas_token_id(),
                },
            },
        ),
        generation: 0,
        gas_price: None,
        sequencer: None,
        sequencer_rollup_address: None,
    }
    .try_into()
    .unwrap();

    let simulation_result = data
        .client()
        .simulate(&types::SimulateBody { body: partial_tx })
        .await
        .unwrap()
        .clone();

    let simulation_result_parsed: SimulateExecutionContainer<S> =
        simulation_result.try_into().unwrap();

    let query_apply_tx_receipt = simulation_result_parsed.apply_tx_result.receipt;

    let result = data
        .runner
        .execute(
            data.user
                .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Burn {
                    coins: sov_bank::Coins {
                        amount: Amount::new(1000),
                        token_id: config_gas_token_id(),
                    },
                }),
        );

    let tx_receipt = result
        .0
        .batch_receipts
        .last()
        .unwrap()
        .tx_receipts
        .last()
        .unwrap();

    assert_eq!(query_apply_tx_receipt.events.len(), tx_receipt.events.len());

    for (simulation_event, tx_event) in query_apply_tx_receipt
        .events
        .iter()
        .zip(tx_receipt.events.iter())
    {
        assert_eq!(simulation_event.key(), tx_event.key());
        assert_eq!(simulation_event.value(), tx_event.value());
    }

    assert!(
        matches!(query_apply_tx_receipt.receipt, TxEffect::Successful(..)),
        "The queries receipt isn't successful. Instead, the receipt is {:?}",
        query_apply_tx_receipt.receipt
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sync_status_fully_synced() {
    let data = TestData::setup().await;

    let expected_synced_da_height = 100;

    data.send_sync_status(SyncStatus::Synced {
        synced_da_height: expected_synced_da_height,
    });

    let sync_status: types::SyncStatus = data.client().get_sync_status().await.unwrap().clone();

    assert_eq!(
        sync_status,
        types::SyncStatus::Synced {
            synced_da_height: expected_synced_da_height as i64
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sync_status_syncing() {
    let data = TestData::setup().await;

    let synced_da_height = 100;
    let target_da_height = 200;

    data.send_sync_status(SyncStatus::Syncing {
        synced_da_height,
        target_da_height,
    });

    let sync_status: types::SyncStatus = data.client().get_sync_status().await.unwrap().clone();

    assert_eq!(
        sync_status,
        types::SyncStatus::Syncing {
            synced_da_height: synced_da_height as i64,
            target_da_height: target_da_height as i64,
        }
    );
}

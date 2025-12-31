use sov_api_spec::types;
use sov_bank::{config_gas_token_id, Bank};
use sov_modules_api::prelude::tokio::{self};
use sov_modules_api::{Amount, Gas, GasArray, GasSpec, Spec, SyncStatus};
use sov_rest_utils::json_obj;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase};

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
        .gas_price;

    let initial_gas_price = S::initial_base_fee_per_gas();

    assert!(
        current_gas_price.dim_is_less_than(initial_gas_price),
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
        api_current_gas_price.dim_is_less_than(api_initial_gas_price),
        "The gas price should have decreased, but it didn't: current gas price {api_current_gas_price}, initial gas price {api_initial_gas_price}"
    );

    assert_eq!(
        api_current_gas_price, current_gas_price,
        "The api gas price should be the same as the current gas price"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulation_success() {
    let data = TestData::setup().await;

    let sender = data.user.credential_id().to_string();
    let receiver = TestUser::<S>::generate_with_default_balance().address();
    let call = sov_bank::CallMessage::<S>::Transfer {
        to: receiver,
        coins: sov_bank::Coins {
            amount: Amount::new(1000),
            token_id: config_gas_token_id(),
        },
    };
    let params = json_obj!({
        "sender": sender,
        "call": {
            "bank": call,
        },
    });
    let client = reqwest::Client::new();

    let response = client
        .post(format!("http://{}/rollup/simulate", data.axum_addr))
        .json(&params)
        .send()
        .await
        .unwrap();
    let actual = response.json::<serde_json::Value>().await.unwrap();

    assert_eq!(actual["outcome"], "success");
    assert!(actual.get("gas_used").is_some());
    assert_eq!(
        actual["events"][0],
        serde_json::json!({
            "key": "Bank/TokenTransferred",
            "module": "Bank",
            "value": {
                "token_transferred": {
                    "from": {
                        "user": data.user.address()
                    },
                    "to": {
                        "user": receiver
                    },
                    "coins": {
                        "amount": "1000",
                        "token_id": "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7"
                    }
                }
            }
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulation_fail() {
    let data = TestData::setup().await;

    let sender = TestUser::<S>::generate_with_default_balance();
    let receiver = TestUser::<S>::generate_with_default_balance().address();
    let call = sov_bank::CallMessage::<S>::Transfer {
        to: receiver,
        coins: sov_bank::Coins {
            amount: Amount::new(1000),
            token_id: config_gas_token_id(),
        },
    };
    let params = json_obj!({
        "sender": sender.credential_id().to_string(),
        "call": {
            "bank": call,
        },
    });
    let client = reqwest::Client::new();

    let response = client
        .post(format!("http://{}/rollup/simulate", data.axum_addr))
        .json(&params)
        .send()
        .await
        .unwrap();
    let actual = response.json::<serde_json::Value>().await.unwrap();
    let expected = serde_json::json!({
        "outcome": "reverted",
        "detail": {
            "call": "transfer_token",
            "error_code": "underflow",
            "base": "0",
            "subtrahend": "1000",
            "message": format!("Insufficient balance for account {}", sender.address()),
        },
    });

    assert_eq!(actual, expected);
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

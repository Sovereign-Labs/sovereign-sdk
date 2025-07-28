use sov_bank::{config_gas_token_id, Coins};
use sov_modules_api::Amount;
use sov_test_utils::runtime::{ApiGetStateData, ApiPath};
use sov_test_utils::{AsUser, AtomicAmount, TransactionTestCase};
use sov_value_setter::ValueSetter;

use crate::helpers::{setup, RT, S};

/// Tests that api routes that are automatically generated work and update as expected.
#[tokio::test(flavor = "multi_thread")]
async fn test_rest_api_routes_default_state() {
    let (user, mut runner) = setup();

    let user_addr = user.address();

    let client = runner.setup_rest_api_server().await;

    let admin_addr_api = runner
        .query_api_response::<ApiGetStateData<String>>(
            &ApiPath::query_module("value-setter").with_default_state_path("admin"),
            &client,
        )
        .await
        .value;

    assert_eq!(
        admin_addr_api,
        Some(user_addr.to_string()),
        "The value returned by the REST API should be the same as the user address"
    );

    let value_api = runner
        .query_api_response::<ApiGetStateData<u64>>(
            &ApiPath::query_module("value-setter").with_default_state_path("value"),
            &client,
        )
        .await
        .value;

    assert_eq!(
        value_api, None,
        "The value returned by the REST API should be `None` because the value is not set"
    );

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 10,
                gas: None,
            },
        ),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    let value_api = runner
        .query_api_response::<ApiGetStateData<u64>>(
            &ApiPath::query_module("value-setter").with_default_state_path("value"),
            &client,
        )
        .await
        .value;

    assert_eq!(
        value_api, Some(10),
        "The value returned by the REST API should be `10` because the value has been set in the previous transaction"
    );
}

/// Ensures that custom API routes work and update as expected.
#[tokio::test(flavor = "multi_thread")]
async fn test_rest_api_routes_custom_api() {
    let (user, mut runner) = setup();

    let user_addr = user.address();
    let initial_user_balance = user.available_gas_balance;

    let client = runner.setup_rest_api_server().await;

    let path = ApiPath::query_module("bank").with_custom_api_path(
        format!("tokens/{}/balances/{}", config_gas_token_id(), user_addr).as_str(),
    );

    let api_user_balance = runner.query_api_response::<Coins>(&path, &client).await;

    assert_eq!(
        api_user_balance.amount, initial_user_balance,
        "The user balance should be the same as the user's available gas balance"
    );

    let gas_used = AtomicAmount::new(Amount::ZERO);
    let gas_used_clone = gas_used.clone();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 10,
                gas: None,
            },
        ),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful());
            gas_used.add(result.gas_value_used);
        }),
    });

    let api_user_balance: Coins = runner.query_api_response(&path, &client).await;

    let expected_balance = initial_user_balance
        .checked_sub(gas_used_clone.get())
        .unwrap();

    assert_eq!(
        api_user_balance.amount, expected_balance,
        "The user balance should be the same as the user's available gas balance minus the gas used to send the transaction"
    );
}

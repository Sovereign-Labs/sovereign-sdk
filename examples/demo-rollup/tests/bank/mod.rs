use std::net::SocketAddr;

use borsh::BorshSerialize;
use demo_stf::genesis_config::GenesisPaths;
use demo_stf::runtime::RuntimeCall;
use jsonrpsee::core::client::{Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use sov_bank::Coins;
use sov_mock_da::MockDaSpec;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Address, PrivateKey, Spec};
use sov_modules_stf_blueprint::kernels::basic::BasicKernelGenesisPaths;
use sov_sequencer::utils::SimpleClient;
use sov_stf_runner::RollupProverConfig;

use crate::test_helpers::start_rollup;

const TOKEN_SALT: u64 = 0;
const TOKEN_NAME: &str = "test_token";

#[tokio::test]
async fn bank_tx_tests() -> Result<(), anyhow::Error> {
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    let rollup_task = tokio::spawn(async {
        start_rollup(
            port_tx,
            GenesisPaths::from_dir("../test-data/genesis/integration-tests"),
            BasicKernelGenesisPaths {
                chain_state: "../test-data/genesis/integration-tests/chain_state.json".into(),
            },
            RollupProverConfig::Execute,
        )
        .await;
    });

    let port = port_rx.await.unwrap();

    // If the rollup throws an error, return it and stop trying to send the transaction
    tokio::select! {
        err = rollup_task => err?,
        res = send_test_bank_txs(port) => res?,
    };
    Ok(())
}

async fn build_create_token_tx(key: &DefaultPrivateKey, nonce: u64) -> Transaction<DefaultContext> {
    let user_address: <DefaultContext as Spec>::Address = key.to_address();
    let msg = RuntimeCall::<DefaultContext, MockDaSpec>::bank(sov_bank::CallMessage::<
        DefaultContext,
    >::CreateToken {
        salt: TOKEN_SALT,
        token_name: TOKEN_NAME.to_string(),
        initial_balance: 1000,
        minter_address: user_address,
        authorized_minters: vec![],
    });
    let chain_id = 0;
    let gas_tip = 0;
    let gas_limit = 0;
    Transaction::<DefaultContext>::new_signed_tx(
        &key,
        msg.try_to_vec().unwrap(),
        chain_id,
        gas_tip,
        gas_limit,
        nonce,
    )
}

async fn build_transfer_token_tx(
    key: &DefaultPrivateKey,
    token_address: Address,
    recipient: <DefaultContext as Spec>::Address,
    amount: u64,
    nonce: u64,
) -> Transaction<DefaultContext> {
    let msg = RuntimeCall::<DefaultContext, MockDaSpec>::bank(sov_bank::CallMessage::<
        DefaultContext,
    >::Transfer {
        to: recipient,
        coins: Coins {
            amount,
            token_address,
        },
    });
    let chain_id = 0;
    let gas_tip = 0;
    let gas_limit = 0;
    Transaction::<DefaultContext>::new_signed_tx(
        &key,
        msg.try_to_vec().unwrap(),
        chain_id,
        gas_tip,
        gas_limit,
        nonce,
    )
}

async fn send_test_bank_txs(rpc_address: SocketAddr) -> Result<(), anyhow::Error> {
    let key = DefaultPrivateKey::generate();
    let user_address: <DefaultContext as Spec>::Address = key.to_address();

    let token_address = sov_bank::get_token_address::<DefaultContext>(
        TOKEN_NAME,
        user_address.as_ref(),
        TOKEN_SALT,
    );

    let tx = build_create_token_tx(&key, 0).await;

    let port = rpc_address.port();
    let client = SimpleClient::new("localhost", port).await?;

    let mut slot_processed_subscription: Subscription<u64> = client
        .ws()
        .subscribe(
            "ledger_subscribeSlots",
            rpc_params![],
            "ledger_unsubscribeSlots",
        )
        .await?;

    client.send_transaction(tx).await?;

    // Wait until the rollup has processed the next slot
    let _ = slot_processed_subscription.next().await;

    let balance_response = sov_bank::BankRpcClient::<DefaultContext>::balance_of(
        client.http(),
        None,
        user_address,
        token_address,
    )
    .await?;
    assert_eq!(balance_response.amount.unwrap_or_default(), 1000);

    let recipient_key = DefaultPrivateKey::generate();
    let recipient_address: <DefaultContext as Spec>::Address = recipient_key.to_address();

    let tx = build_transfer_token_tx(
        &key,
        token_address.clone(),
        recipient_address.clone(),
        100,
        1,
    )
    .await;

    let mut slot_processed_subscription: Subscription<u64> = client
        .ws()
        .subscribe(
            "ledger_subscribeSlots",
            rpc_params![],
            "ledger_unsubscribeSlots",
        )
        .await?;

    client.send_transaction(tx).await?;

    // Wait until the rollup has processed the next slot
    let _ = slot_processed_subscription.next().await;

    let balance_response = sov_bank::BankRpcClient::<DefaultContext>::balance_of(
        client.http(),
        None,
        user_address,
        token_address,
    )
    .await?;
    assert_eq!(balance_response.amount.unwrap_or_default(), 900);

    let tx = build_transfer_token_tx(
        &key,
        token_address.clone(),
        recipient_address.clone(),
        200,
        2,
    )
    .await;

    let mut slot_processed_subscription: Subscription<u64> = client
        .ws()
        .subscribe(
            "ledger_subscribeSlots",
            rpc_params![],
            "ledger_unsubscribeSlots",
        )
        .await?;

    client.send_transaction(tx).await?;

    // Wait until the rollup has processed the next slot
    let _ = slot_processed_subscription.next().await;

    let balance_response = sov_bank::BankRpcClient::<DefaultContext>::balance_of(
        client.http(),
        None,
        user_address,
        token_address,
    )
    .await?;
    assert_eq!(balance_response.amount.unwrap_or_default(), 700);

    let balance_response = sov_bank::BankRpcClient::<DefaultContext>::balance_of(
        client.http(),
        Some(3),
        user_address,
        token_address,
    )
    .await?;
    assert_eq!(balance_response.amount.unwrap_or_default(), 900);

    let balance_response = sov_bank::BankRpcClient::<DefaultContext>::balance_of(
        client.http(),
        Some(4),
        user_address,
        token_address,
    )
    .await?;
    assert_eq!(balance_response.amount.unwrap_or_default(), 700);

    let balance_response = sov_bank::BankRpcClient::<DefaultContext>::balance_of(
        client.http(),
        Some(2),
        user_address,
        token_address,
    )
    .await?;
    assert_eq!(balance_response.amount.unwrap_or_default(), 1000);

    Ok(())
}

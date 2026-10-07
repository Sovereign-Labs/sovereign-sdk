//! Tests for clients that send the `x-sov-sealed-blocks-only` header, such as block explorers.
//!
//! These clients must see the newest sealed block as the chain head and never see synthetic
//! blocks, so that soft-confirmed transactions don't show up as a stream of reorgs.

use std::net::SocketAddr;

use alloy::transports::http::reqwest::header::{HeaderMap, HeaderValue};
use alloy_primitives::{Address, TxHash};
use alloy_provider::ext::DebugApi;
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::BlockNumberOrTag::{Finalized, Latest, Number};
use alloy_rpc_types_eth::{Block, Filter, Header};
use alloy_rpc_types_trace::geth::{CallConfig, GethDebugTracingOptions};
use jsonrpsee::core::client::{Subscription, SubscriptionClientT};
use jsonrpsee::rpc_params;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use sov_eth_client::SimpleStorageClient;
use sov_rest_utils::SEALED_BLOCKS_ONLY_HEADER;
use sov_rpc_eth_types::EthApiError;
use sov_test_utils::test_rollup::TestRollup;

use crate::common::{
    alloy_client, alloy_client_with_reqwest, poll_until, setup_with_simple_storage, EVM_EXTENSION,
    SENDER_PRIV_KEY,
};
use crate::runtime::EvmBlueprint;

fn sealed_only_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        SEALED_BLOCKS_ONLY_HEADER.as_str(),
        HeaderValue::from_static("true"),
    );
    headers
}

fn sealed_only_client(socket: SocketAddr) -> DynProvider {
    alloy_client_with_reqwest(
        socket,
        |builder| {
            builder
                .default_headers(sealed_only_headers())
                .build()
                .unwrap()
        },
        SENDER_PRIV_KEY,
    )
}

/// A rollup with a deployed contract and paused batch production.
struct PausedRollup {
    rollup: TestRollup<EvmBlueprint>,
    client: SimpleStorageClient,
    contract_address: Address,
    /// The newest sealed block at the time batch production was paused.
    sealed_head: Block,
}

async fn setup_paused_rollup() -> anyhow::Result<PausedRollup> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract_address = client.alloy_deploy_contract().await;
    rollup.pause_preferred_batches_and_wait().await?;
    // With zero finalization delay, the finalized block is the newest sealed block.
    let sealed_head = alloy_client(rollup.http_addr)
        .get_block_by_number(Finalized)
        .await?
        .expect("finalized block should exist");
    Ok(PausedRollup {
        rollup,
        client,
        contract_address,
        sealed_head,
    })
}

/// Sends a log-emitting tx and waits until it's soft-confirmed, while its block is not sealed.
async fn send_unsealed_tx(paused: &PausedRollup) -> TxHash {
    let tx_hash = paused
        .client
        .alloy_emit_logs(paused.contract_address, 0, 2)
        .await;
    paused.client.wait_for_receipt(tx_hash).await;
    tx_hash
}

#[tokio::test(flavor = "multi_thread")]
async fn latest_block_is_newest_sealed_block() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    send_unsealed_tx(&paused).await;

    let latest = sealed_only_client(paused.rollup.http_addr)
        .get_block_by_number(Latest)
        .await?
        .expect("latest block should exist");

    assert_eq!(
        latest.header.hash, paused.sealed_head.header.hash,
        "latest should be the newest sealed block, not the synthetic block of the in-progress batch"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn block_number_is_newest_sealed_block() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    send_unsealed_tx(&paused).await;

    let block_number = sealed_only_client(paused.rollup.http_addr)
        .get_block_number()
        .await?;

    assert_eq!(block_number, paused.sealed_head.header.number);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unsealed_tx_is_reported_as_pending() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    let tx_hash = send_unsealed_tx(&paused).await;

    let tx = sealed_only_client(paused.rollup.http_addr)
        .get_transaction_by_hash(tx_hash)
        .await?
        .expect("unsealed tx should be returned as pending");

    assert_eq!(
        (tx.block_hash, tx.block_number, tx.transaction_index),
        (None, None, None),
        "a pending tx has no block context"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unsealed_tx_has_no_receipt() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    let tx_hash = send_unsealed_tx(&paused).await;

    let receipt = sealed_only_client(paused.rollup.http_addr)
        .get_transaction_receipt(tx_hash)
        .await?;

    assert_eq!(receipt, None);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn receipt_has_sealed_block_hash_once_sealed() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    let tx_hash = send_unsealed_tx(&paused).await;
    let client = sealed_only_client(paused.rollup.http_addr);

    paused.rollup.resume_preferred_batches().await;
    let receipt = poll_until(
        || async { Ok(client.get_transaction_receipt(tx_hash).await?) },
        |receipt| receipt.is_some(),
        "receipt did not appear after the batch was sealed",
    )
    .await?
    .expect("checked by poll_until");
    let block_number = receipt
        .block_number
        .expect("sealed receipt has a block number");
    let block = client
        .get_block_by_number(Number(block_number))
        .await?
        .expect("sealed block should exist");

    assert_eq!(receipt.block_hash, Some(block.header.hash));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_excludes_unsealed_txs() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    send_unsealed_tx(&paused).await;
    let filter = Filter::new()
        .address(paused.contract_address)
        .from_block(paused.sealed_head.header.number)
        .to_block(Latest);

    let logs = sealed_only_client(paused.rollup.http_addr)
        .get_logs(&filter)
        .await?;

    assert_eq!(logs, vec![]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn synthetic_block_hash_is_unknown() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    send_unsealed_tx(&paused).await;
    // Without the header, `latest` is the synthetic block of the in-progress batch.
    let synthetic_hash = alloy_client(paused.rollup.http_addr)
        .get_block_by_number(Latest)
        .await?
        .expect("latest block should exist")
        .header
        .hash;

    let block = sealed_only_client(paused.rollup.http_addr)
        .get_block_by_hash(synthetic_hash)
        .await?;

    assert_eq!(block, None, "synthetic block hashes should be unknown");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fee_history_newest_block_is_newest_sealed_block() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    send_unsealed_tx(&paused).await;

    let fee_history = sealed_only_client(paused.rollup.http_addr)
        .get_fee_history(1, Latest, &[])
        .await?;

    assert_eq!(fee_history.oldest_block, paused.sealed_head.header.number);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn trace_of_unsealed_tx_is_tx_not_found() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    let tx_hash = send_unsealed_tx(&paused).await;

    let error = sealed_only_client(paused.rollup.http_addr)
        .debug_trace_transaction(
            tx_hash,
            GethDebugTracingOptions::call_tracer(CallConfig::default()),
        )
        .await
        .expect_err("unsealed tx should not be traceable");

    assert_eq!(
        error.as_error_resp().map(|resp| resp.message.to_string()),
        Some(EthApiError::PrunedHistoryUnavailable.to_string()),
        "an unsealed tx should be reported like an unknown tx"
    );
    Ok(())
}

async fn subscribe_new_heads(
    socket: SocketAddr,
    headers: HeaderMap,
) -> anyhow::Result<(WsClient, Subscription<Header>)> {
    let ws_client = WsClientBuilder::default()
        .set_headers(headers)
        .build(format!("ws://{socket}/rpc"))
        .await?;
    let heads = ws_client
        .subscribe::<Header, _>("eth_subscribe", rpc_params!["newHeads"], "eth_unsubscribe")
        .await?;
    Ok((ws_client, heads))
}

#[tokio::test(flavor = "multi_thread")]
async fn new_heads_subscription_only_notifies_sealed_blocks() -> anyhow::Result<()> {
    let paused = setup_paused_rollup().await?;
    let socket = paused.rollup.http_addr;
    // Subscribe with the header first, so it's never throttled for longer than the reference subscription.
    let (_sealed_ws, mut sealed_heads) = subscribe_new_heads(socket, sealed_only_headers()).await?;
    let (_reference_ws, mut reference_heads) =
        subscribe_new_heads(socket, HeaderMap::new()).await?;

    send_unsealed_tx(&paused).await;
    // Without the header, the unsealed tx is notified as a synthetic block. Wait for it, so the
    // sealed-only subscription has had the same chance to (wrongly) notify it.
    reference_heads.next().await.expect("subscription closed")?;
    paused.rollup.resume_preferred_batches().await;
    let first_head = sealed_heads.next().await.expect("subscription closed")?;
    // Look the block up once it's sealed: before that, its number resolves to the synthetic block.
    let sealed_only = sealed_only_client(socket);
    let sealed_block = poll_until(
        || async {
            Ok(sealed_only
                .get_block_by_number(Number(first_head.number))
                .await?)
        },
        |block| block.is_some(),
        "notified block was never sealed",
    )
    .await?
    .expect("checked by poll_until");

    assert_eq!(
        first_head.hash, sealed_block.header.hash,
        "the first notification should be a sealed block, not a synthetic one"
    );
    Ok(())
}

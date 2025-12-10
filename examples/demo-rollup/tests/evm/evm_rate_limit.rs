use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy_primitives::Address;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;

use crate::evm::evm_test_helper::alloy_client_with_reqwest;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_rate_limit() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("123.123.123.123"),
    );

    let b = |b: reqwest::ClientBuilder| b.default_headers(headers).build().unwrap();

    let client = alloy_client_with_reqwest(rollup.http_addr, b);

    let tx = TransactionRequest::default().with_to(Address::ZERO);
    let pending = client.send_transaction(tx).await?;
    let hash = *pending.tx_hash();

    let confirmed_hash = pending.watch().await?;

    assert_eq!(confirmed_hash, hash);
    Ok(())
}

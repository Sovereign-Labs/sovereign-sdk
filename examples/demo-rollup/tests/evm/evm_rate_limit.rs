use crate::evm::evm_test_helper::alloy_client_with_reqwest;
use crate::evm::evm_test_helper::start_node;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SECONDARY_SENDER_PRIV_KEY;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy_primitives::Address;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockDemoRollup;
use sov_demo_rollup::MockRollupSpec;
use sov_modules_api::execution_mode::Native;
use sov_sequencer::SovRateLimiterConfig;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use sov_test_utils::test_rollup::TestRollup;

async fn setup_test_rollup(
    rate_limiter: SovRateLimiterConfig,
) -> TestRollup<MockDemoRollup<Native>> {
    let host_args = mock_da_risc0_host_args();
    let config = get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(host_args);
    start_node(config, 0, Some(EVM_EXTENSION), None, Some(rate_limiter)).await
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_rate_limit() -> anyhow::Result<()> {
    let rate_limiter = SovRateLimiterConfig {
        max_requests_per_batch: 0,
        refill_rate: 0,
    };

    let rollup = setup_test_rollup(rate_limiter).await;
    rollup.wait_for_next_blocks(1).await;

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("123.123.123.123"),
    );

    {
        let b = |b: reqwest::ClientBuilder| b.default_headers(headers).build().unwrap();

        let client = alloy_client_with_reqwest(rollup.http_addr, b, SENDER_PRIV_KEY);
        let tx = TransactionRequest::default().with_to(Address::ZERO);
        let pending = client.send_transaction(tx).await?;
        let hash = *pending.tx_hash();
        let confirmed_hash = pending.watch().await?;
        assert_eq!(confirmed_hash, hash);
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("123.123.123.123"),
    );
    {
        let b = |b: reqwest::ClientBuilder| b.default_headers(headers).build().unwrap();

        let client = alloy_client_with_reqwest(rollup.http_addr, b, SECONDARY_SENDER_PRIV_KEY);
        let _tx = TransactionRequest::default().with_to(Address::ZERO);
        //let _pending = client.send_transaction(tx).await.unwrap();
    }
    Ok(())
}

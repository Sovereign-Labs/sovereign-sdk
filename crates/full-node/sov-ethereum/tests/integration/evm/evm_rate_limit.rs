use core::net::SocketAddr;

use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy::transports::{RpcError, TransportErrorKind};
use alloy_primitives::Address;
use alloy_provider::DynProvider;
use alloy_rpc_types_eth::TransactionInput;
use reqwest::header::{HeaderMap, HeaderValue};
use sov_full_node_configs::sequencer::Limits;
use sov_rollup_interface::common::RollupHeight;
use sov_sequencer::SovRateLimiterConfig;

use crate::common::{
    alloy_client_with_reqwest, setup_test_rollup_with_rate_limiter, EVM_EXTENSION,
    SECONDARY_SENDER_PRIV_KEY, SENDER_PRIV_KEY,
};

const X_FORWARDED_FOR: &str = "123.123.123.123";

fn make_client_with_x_forwarded_for_header(http_addr: SocketAddr, priv_key: &str) -> DynProvider {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_static(X_FORWARDED_FOR));
    let client_builder = |builder: alloy::transports::http::reqwest::ClientBuilder| {
        builder.default_headers(headers).build().unwrap()
    };
    alloy_client_with_reqwest(http_addr, client_builder, priv_key)
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_rate_limit() -> anyhow::Result<()> {
    let rate_limiter = SovRateLimiterConfig {
        default_limits: Limits {
            resources_per_bucket: 1,
            refill_rate: 0,
        },
        max_nb_of_concurrent_users_in_rate_limiter: 1000,
        max_requests_per_second: 1,
        address_custom_limits: Vec::default(),
        height_for_gas_limit_computation: RollupHeight::GENESIS,
        ip_custom_limits: Vec::default(),
    };

    let rollup = setup_test_rollup_with_rate_limiter(0, EVM_EXTENSION, rate_limiter).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    // Make first request.
    {
        let client = make_client_with_x_forwarded_for_header(rollup.http_addr, SENDER_PRIV_KEY);
        let mut tx = TransactionRequest::default().with_to(Address::ZERO);
        // Set some input to cross the allowed space limit.
        tx.input = TransactionInput {
            input: Some(vec![1; 10000].into()),
            data: None,
        };
        let pending = client.send_transaction(tx).await?;
        _ = pending.watch().await?;
    }

    // The second request should be rate-limited, and the error message must include `X_FORWARDED_FOR`.
    {
        let client =
            make_client_with_x_forwarded_for_header(rollup.http_addr, SECONDARY_SENDER_PRIV_KEY);
        let tx = TransactionRequest::default().with_to(Address::ZERO);
        let err = client.send_transaction(tx).await.unwrap_err();
        assert_err(err);
    }
    Ok(())
}

fn assert_err(err: RpcError<TransportErrorKind>) {
    let payload = err.as_error_resp().unwrap();
    assert!(
        payload.message.as_ref().contains(X_FORWARDED_FOR),
        "expected error message to include IP: {X_FORWARDED_FOR}, but it was: {}",
        payload.message,
    );
}

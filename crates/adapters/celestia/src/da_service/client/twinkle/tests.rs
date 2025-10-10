use super::*;
use crate::config::TwinkleConfig;
use crate::test_helper::{ADDR_1, ROLLUP_PROOF_NAMESPACE};
use crate::verifier::RollupParams;
use crate::CelestiaConfig;
use crate::CelestiaService;
use celestia_types::nmt::Namespace;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use std::str::FromStr;

const BATCH_NAMESPACE: Namespace = Namespace::const_v0(*b"sov-twinkl");
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const POOL_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const PULL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn build_client() -> TwinkleClient {
    let backoff_policy = ExponentialBuilder::default();
    let config = TwinkleConfig::test();
    let req_client =
        config.construct_reqwest_client(REQUEST_TIMEOUT, CONNECT_TIMEOUT, POOL_IDLE_TIMEOUT);
    TwinkleClient::new(
        req_client,
        config.network,
        PULL_INTERVAL,
        TOTAL_TIMEOUT,
        backoff_policy,
        Some(FeePriority::Fast),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn async_blob_submit() -> anyhow::Result<()> {
    sov_test_utils::logging::initialize_or_change_logging_with_filter(
        "debug,hyper=info,sov_celestia_adapter=trace",
    );

    let twinkle_client = build_client();

    let celestia_address = CelestiaAddress::from_str(ADDR_1)?;

    let blob: Vec<u8> = b"hello-from-sov-rust".to_vec();

    let start = std::time::Instant::now();
    let rx = twinkle_client
        .submit_blob_to_namespace_inner(
            &blob,
            RollupNamespace::Batch(BATCH_NAMESPACE),
            &celestia_address,
        )
        .await;
    println!("A: {:?}", start.elapsed());
    let res = rx.await?;
    println!("B: {:?}", start.elapsed());
    let receipt = res?;
    println!("RECEIPT: {receipt:?}");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_head_block_header() -> anyhow::Result<()> {
    let config = CelestiaConfig::dev_config("http://127.0.0.1:26658");
    let params = RollupParams {
        rollup_batch_namespace: BATCH_NAMESPACE,
        rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
    };

    let standard_client = CelestiaService::new(config, params).await;
    let twinkle_client = build_client();

    let standard_header = standard_client.get_head_block_header().await?;
    println!("Standard Header {standard_header:?}");

    let height = standard_header.height();
    let twinkle_header = twinkle_client.get_block_header_at(height).await?;
    println!("-------- {height}");
    println!("Twinkle Header {twinkle_header:?}");
    println!("Twinkle DAH: {:?}", twinkle_header.dah);

    assert_eq!(twinkle_header.header, standard_header.header);
    assert_eq!(twinkle_header.dah, standard_header.dah);

    Ok(())
}

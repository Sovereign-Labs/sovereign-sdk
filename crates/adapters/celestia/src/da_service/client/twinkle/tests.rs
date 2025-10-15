use super::*;
use crate::config::TwinkleConfig;
use crate::test_helper::{ADDR_1, ROLLUP_PROOF_NAMESPACE};
use crate::verifier::RollupParams;
use crate::CelestiaConfig;
use crate::CelestiaService;
use celestia_rpc::share::ShareClient;
use celestia_rpc::HeaderClient;
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
        CelestiaAddress::from_str(ADDR_1).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn async_blob_submit() -> anyhow::Result<()> {
    sov_test_utils::logging::initialize_or_change_logging_with_filter(
        "debug,hyper=info,sov_celestia_adapter=trace",
    );

    let twinkle_client = build_client();

    let blob: Vec<u8> = b"hello-from-sov-rust".to_vec();

    let start = std::time::Instant::now();
    let rx = twinkle_client
        .submit_blob_to_namespace(&blob, RollupNamespace::Batch(BATCH_NAMESPACE))
        .await;
    println!("A: {:?}", start.elapsed());
    let res = rx.await?;
    println!("B: {:?}", start.elapsed());
    let receipt = res?;
    println!("RECEIPT: {receipt:?}");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn get_block_header() -> anyhow::Result<()> {
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

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_get_head_block_header() -> anyhow::Result<()> {
    let twinkle_client = build_client();

    let twinkle_header = twinkle_client.get_head_block_header().await?;
    println!("Twinkle Header {twinkle_header:?}");
    println!("Twinkle DAH: {:?}", twinkle_header.dah);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_namespace_data() -> anyhow::Result<()> {
    let twinkle_client = build_client();

    let namespace = RollupNamespace::Batch(Namespace::const_v0(*b"sov-mini-i"));
    let height = 8410087;
    let twinkle_response = twinkle_client
        .get_namespace_data(&namespace, height)
        .await?;
    println!("Twinkle ROWS: {}", twinkle_response.rows.len());
    let vanilla_raw_client =
        jsonrpsee::http_client::HttpClientBuilder::default().build("http://127.0.0.1:26658")?;

    let block_header = vanilla_raw_client.header_get_by_height(height).await?;
    let rollup_batch_rows = vanilla_raw_client
        .share_get_namespace_data(&block_header, namespace.id())
        .await?;
    assert_eq!(twinkle_response, rollup_batch_rows);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_blobs_at() -> anyhow::Result<()> {
    let twinkle_client = build_client();

    let namespace = RollupNamespace::Batch(Namespace::const_v0(*b"sov-mini-i"));
    let height = 8410087;
    let twinkle_response = twinkle_client.get_blobs_at(height, &namespace).await?;
    println!("BLOBS: {}", twinkle_response.len());
    Ok(())
}

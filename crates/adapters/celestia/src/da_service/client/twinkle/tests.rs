use super::*;
use crate::test_helper::{ADDR_1, ROLLUP_PROOF_NAMESPACE};
use crate::verifier::RollupParams;
use crate::CelestiaConfig;
use crate::CelestiaService;
use celestia_types::nmt::Namespace;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use std::str::FromStr;

const API_KEY: &str = "TEMP_SECRET";

fn default_mocha_config() -> TwinkleConfig {
    TwinkleConfig {
        api_key: Some(API_KEY.to_string()),
        network: Network::Mocha,
        pull_interval_millis: 100,
        request_timeout_secs: 30,
        total_timeout_secs: 300,
    }
}

const BATCH_NAMESPACE: Namespace = Namespace::const_v0(*b"sov-twinkl");

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn async_blob_submit() -> anyhow::Result<()> {
    sov_test_utils::logging::initialize_or_change_logging_with_filter(
        "debug,hyper=info,sov_celestia_adapter=trace",
    );
    let backoff_policy = ExponentialBuilder::default();
    let twinkle_client = TwinkleClient::new(&default_mocha_config(), backoff_policy)?;
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
#[ignore]
async fn get_head_block_header() -> anyhow::Result<()> {
    let config = CelestiaConfig::dev_config("http://127.0.0.1:26658");
    let params = RollupParams {
        rollup_batch_namespace: BATCH_NAMESPACE,
        rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
    };

    let vanilla_client = CelestiaService::new(config, params).await;

    let backoff_policy = ExponentialBuilder::default();
    let twinkle_client = TwinkleClient::new(&default_mocha_config(), backoff_policy)?;

    let twinkle_header = twinkle_client.get_head_block_header().await?;
    println!("Twinkle Header {twinkle_header:?}");

    let height = twinkle_header.height();

    let vanilla_header = vanilla_client.get_block_header_at(height).await?;

    assert_eq!(twinkle_header.header, vanilla_header.header);

    Ok(())
}

use std::fs;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber;

use sov_avail_adapter::service::AvailDAService;
use sov_avail_adapter::types::config::AvailDAConfig;
use sov_rollup_interface::node::da::DaService;

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info") // change to "debug" or "trace" if needed
        .with_target(false)
        .try_init();
}

fn load_config() -> AvailDAConfig {
    let path = PathBuf::from("avail_da_config.toml");
    let raw = fs::read_to_string(&path).expect("failed to read avail_da_config.toml");
    toml::from_str(&raw).expect("invalid AvailDAConfig TOML")
}

async fn init_service() -> AvailDAService {
    init_tracing();
    let config = load_config();
    AvailDAService::new_from_config(config).await.unwrap()
}

#[tokio::test]
async fn test_get_block_at() {
    let service = init_service().await;

    let height: u64 = 1_000_000; // pick a known testnet height
    let block = service.get_block_at(height).await;
    assert!(block.is_ok(), "Expected block, got {:?}", block);
    assert_eq!(
        block.as_ref().unwrap().header.header.number,
        height as u32,
        "Block height mismatch"
    );
    info!(
        "Block at {}: {:?}",
        height,
        block.unwrap().header.header.number
    );
}

#[tokio::test]
async fn test_get_last_finalized_block_header() {
    let service = init_service().await;

    let header = service.get_last_finalized_block_header().await;
    assert!(
        header.is_ok(),
        "Expected finalized header, got {:?}",
        header
    );
    info!("Finalized header: {:?}", header.unwrap().header.number);
}

#[tokio::test]
async fn test_get_head_block_header() {
    let service = init_service().await;

    let header = service.get_head_block_header().await;
    assert!(header.is_ok(), "Expected head header, got {:?}", header);
    info!("Head header: {:?}", header.unwrap().header.number);
}

#[tokio::test]
async fn test_send_transaction_and_proof() {
    let service = init_service().await;

    // Dummy blob
    let blob = b"hello from test blob";
    let proof_blob = b"hello from test proof";

    // Send transaction
    let rx_tx = service.send_transaction(blob).await;
    let result_tx = rx_tx.await.unwrap();
    assert!(
        result_tx.is_ok(),
        "Transaction submission failed: {:?}",
        result_tx
    );
    info!("Transaction submitted: {:?}", result_tx.unwrap());

    // Send proof
    let rx_proof = service.send_proof(proof_blob).await;
    let result_proof = rx_proof.await.unwrap();
    assert!(
        result_proof.is_ok(),
        "Proof submission failed: {:?}",
        result_proof
    );
    info!("Proof submitted: {:?}", result_proof.unwrap());
}

#[tokio::test]
async fn test_get_proofs_at_not_implemented() {
    let service = init_service().await;

    let result = service.get_proofs_at(100).await;
    assert!(result.is_err(), "Expected unimplemented error");
    info!("Got expected error: {:?}", result.unwrap_err());
}

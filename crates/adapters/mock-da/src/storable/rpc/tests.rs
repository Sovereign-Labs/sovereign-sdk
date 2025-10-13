use super::client::StorableMockDaClient;
use crate::storable::layer::StorableMockDaLayer;
use crate::storable::rpc::server::start_server;
use crate::storable::StorableMockDaService;
use crate::MockAddress;
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use std::sync::Arc;
use tokio::sync::RwLock;

async fn create_da() -> StorableMockDaService {
    let da_layer = Arc::new(RwLock::new(
        StorableMockDaLayer::new_in_memory(0)
            .await
            .expect("Failed to create DA layer"),
    ));
    StorableMockDaService::new_manual_producing(MockAddress::new([1; 32]), da_layer).await
}

#[tokio::test]
async fn test_get_head_block_header() {
    let da_service = create_da().await;
    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();
    let client = StorableMockDaClient::new(format!("http://{addr}"));

    da_service.produce_block_now().await.unwrap();
    let header = client.get_head_block_header().await.unwrap();
    assert_eq!(header.height(), 1);

    da_service.produce_block_now().await.unwrap();
    let header = client.get_head_block_header().await.unwrap();
    assert_eq!(header.height(), 2);
}

#[tokio::test]
async fn test_send_transaction() {
    let da_service = create_da().await;
    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    let client = StorableMockDaClient::new(format!("http://{addr}"));

    let test_blob = b"test blob data";
    let receiver = client.send_transaction(test_blob).await;
    let result = receiver.await.unwrap();
    da_service.produce_block_now().await.unwrap();

    let block = client.get_block_at(0).await.unwrap();
    assert_eq!(block.batch_blobs.len(), 0);

    let mut block = client.get_block_at(1).await.unwrap();
    assert_eq!(block.batch_blobs.len(), 1);
    assert_eq!(block.batch_blobs[0].full_data(), test_blob);
    assert!(result.is_ok());
}

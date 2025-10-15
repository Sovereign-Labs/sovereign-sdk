use crate::da_service::client::standard_node::StandardNodeClient;
use crate::da_service::client::twinkle::TwinkleClient;
use crate::types::{FilteredCelestiaBlock, RollupNamespace, TmHash};
use crate::verifier::address::CelestiaAddress;
use crate::CelestiaHeader;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use tokio::sync::oneshot;

pub mod standard_node;
pub mod twinkle;

#[derive(Debug, Clone)]
pub enum CelestiaClient {
    StandardNode(StandardNodeClient),
    Twinkle(TwinkleClient),
}

impl CelestiaClient {
    pub async fn submit_blob_to_namespace(
        &self,
        blob: &[u8],
        namespace: RollupNamespace,
    ) -> oneshot::Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        match self {
            CelestiaClient::StandardNode(client) => {
                client.submit_blob_to_namespace(blob, namespace).await
            }
            CelestiaClient::Twinkle(client) => {
                client.submit_blob_to_namespace(blob, namespace).await
            }
        }
    }

    pub async fn get_block_header_at(&self, height: u64) -> anyhow::Result<CelestiaHeader> {
        match self {
            CelestiaClient::StandardNode(client) => client.get_block_header_at(height).await,
            CelestiaClient::Twinkle(client) => client.get_block_header_at(height).await,
        }
    }

    pub async fn get_head_block_header(&self) -> anyhow::Result<CelestiaHeader> {
        match self {
            CelestiaClient::StandardNode(client) => client.get_head_block_header().await,
            CelestiaClient::Twinkle(client) => client.get_head_block_header().await,
        }
    }

    pub async fn get_block_at(
        &self,
        height: u64,
        batch_namespace: &RollupNamespace,
        proof_namespace: &RollupNamespace,
    ) -> anyhow::Result<FilteredCelestiaBlock> {
        match self {
            CelestiaClient::StandardNode(client) => {
                client
                    .get_block_at(height, batch_namespace, proof_namespace)
                    .await
            }
            CelestiaClient::Twinkle(client) => {
                client
                    .get_block_at(height, batch_namespace, proof_namespace)
                    .await
            }
        }
    }

    pub async fn get_blobs_at(
        &self,
        height: u64,
        namespace: &RollupNamespace,
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        match self {
            CelestiaClient::StandardNode(client) => client.get_blobs_at(height, namespace).await,
            CelestiaClient::Twinkle(client) => client.get_blobs_at(height, namespace).await,
        }
    }

    pub fn get_signer(&self) -> CelestiaAddress {
        match self {
            CelestiaClient::StandardNode(client) => client.signer_address.clone(),
            CelestiaClient::Twinkle(client) => client.signer_address.clone(),
        }
    }
}

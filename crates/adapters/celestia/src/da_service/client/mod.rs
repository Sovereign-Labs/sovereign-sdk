use crate::da_service::client::standard_node::StandardNodeClient;
use crate::da_service::client::twinkle::TwinkleClient;
use crate::types::{RollupNamespace, TmHash};
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
        signer: &CelestiaAddress,
    ) -> oneshot::Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        match self {
            CelestiaClient::StandardNode(client) => {
                client
                    .submit_blob_to_namespace(blob, namespace, signer)
                    .await
            }
            CelestiaClient::Twinkle(client) => {
                client
                    .submit_blob_to_namespace(blob, namespace, signer)
                    .await
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
}

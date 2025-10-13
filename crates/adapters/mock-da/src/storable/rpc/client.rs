#![allow(dead_code, missing_docs)]

use async_trait::async_trait;
use sov_rollup_interface::da::{DaSpec, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{DaService, SubmitBlobReceipt};
use tokio::sync::oneshot;

use super::types::*;
use crate::{MockBlock, MockDaConfig, MockDaSpec, MockDaVerifier};

#[derive(Clone)]
pub struct StorableMockDaClient {
    base_url: String,
    client: reqwest::Client,
}

impl StorableMockDaClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[async_trait]
impl DaService for StorableMockDaClient {
    type Spec = MockDaSpec;
    type Config = MockDaConfig;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    const GUARANTEES_TRANSACTION_ORDERING: bool = true;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let url = self.url(&format!("/blocks/{}", height));
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            let error: ErrorResponse = response.json().await?;
            return Err(anyhow::anyhow!("Server error: {}", error.error));
        }

        let block_response: BlockResponse = response.json().await?;
        Ok(block_response.block)
    }

    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let url = self.url(&format!("/block-headers/{}", height));
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            let error: ErrorResponse = response.json().await?;
            return Err(anyhow::anyhow!("Server error: {}", error.error));
        }

        let header_response: BlockHeaderResponse = response.json().await?;
        Ok(header_response.header)
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let url = self.url("/finalized-block-header");
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            let error: ErrorResponse = response.json().await?;
            return Err(anyhow::anyhow!("Server error: {}", error.error));
        }

        let header_response: BlockHeaderResponse = response.json().await?;
        Ok(header_response.header)
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let url = self.url("/head-block-header");
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            let error: ErrorResponse = response.json().await?;
            return Err(anyhow::anyhow!("Server error: {}", error.error));
        }

        let header_response: BlockHeaderResponse = response.json().await?;
        Ok(header_response.header)
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        // For the HTTP client, we use the block's implementation directly
        // since this method is not async
        block.as_relevant_blobs()
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
        let url = self.url("/extraction-proof");
        let request = ExtractionProofRequest {
            block: block.clone(),
            blobs: blobs.clone(),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .expect("Failed to send extraction proof request");

        if !response.status().is_success() {
            let error: ErrorResponse = response
                .json()
                .await
                .expect("Failed to parse error response");
            panic!("Server error: {}", error.error);
        }

        let proof_response: ExtractionProofResponse = response
            .json()
            .await
            .expect("Failed to parse extraction proof response");
        proof_response.proofs
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();

        let url = self.url("/send-transaction");
        let request = SubmitTransactionRequest {
            blob: hex::encode(blob),
        };

        let client = self.client.clone();
        tokio::spawn(async move {
            let result = async {
                let response = client.post(&url).json(&request).send().await?;

                if !response.status().is_success() {
                    let error: ErrorResponse = response.json().await?;
                    return Err(anyhow::anyhow!("Server error: {}", error.error));
                }

                let submit_response: SubmitBlobResponse = response.json().await?;
                Ok(submit_response.receipt)
            }
            .await;

            let _ = tx.send(result);
        });

        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();

        let url = self.url("/send-proof");
        let request = SubmitProofRequest {
            aggregated_proof_data: hex::encode(aggregated_proof_data),
        };

        let client = self.client.clone();
        tokio::spawn(async move {
            let result = async {
                let response = client.post(&url).json(&request).send().await?;

                if !response.status().is_success() {
                    let error: ErrorResponse = response.json().await?;
                    return Err(anyhow::anyhow!("Server error: {}", error.error));
                }

                let submit_response: SubmitBlobResponse = response.json().await?;
                Ok(submit_response.receipt)
            }
            .await;

            let _ = tx.send(result);
        });

        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        let url = self.url(&format!("/proofs/{}", height));
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            let error: ErrorResponse = response.json().await?;
            return Err(anyhow::anyhow!("Server error: {}", error.error));
        }

        let proofs_response: ProofsResponse = response.json().await?;
        let proofs = proofs_response
            .proofs
            .into_iter()
            .map(|hex_proof| hex::decode(hex_proof))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(proofs)
    }

    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address {
        let url = self.url("/signer");
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .expect("Failed to get signer");

        if !response.status().is_success() {
            let error: ErrorResponse = response
                .json()
                .await
                .expect("Failed to parse error response");
            panic!("Server error: {}", error.error);
        }

        let signer_response: SignerResponse = response
            .json()
            .await
            .expect("Failed to parse signer response");
        signer_response.address
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storable::layer::StorableMockDaLayer;
    use crate::storable::rpc::server::create_router;
    use crate::storable::rpc::server::start_server;
    use crate::storable::StorableMockDaService;
    use crate::MockAddress;
    use sov_rollup_interface::da::BlockHeaderTrait;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn test_client_with_real_server_get_head_block_header() {
        // Create a server
        let da_layer = Arc::new(RwLock::new(
            StorableMockDaLayer::new_in_memory(0)
                .await
                .expect("Failed to create DA layer"),
        ));
        let da_service =
            StorableMockDaService::new_manual_producing(MockAddress::new([1; 32]), da_layer).await;

        let addr = start_server(da_service).await;

        // Create a client
        let client = StorableMockDaClient::new(format!("http://{}", addr));

        // Test the client
        let header = client.get_head_block_header().await.unwrap();
        assert_eq!(header.height(), 0); // Genesis block
    }

    #[tokio::test]
    async fn test_client_with_real_server_send_transaction() {
        // Create a server
        let da_layer = Arc::new(RwLock::new(
            StorableMockDaLayer::new_in_memory(0)
                .await
                .expect("Failed to create DA layer"),
        ));
        let da_service =
            StorableMockDaService::new_manual_producing(MockAddress::new([1; 32]), da_layer).await;

        let addr = start_server(da_service).await;

        // Create a client
        let client = StorableMockDaClient::new(format!("http://{}", addr));

        // Test the client
        let test_blob = b"test blob data";
        let receiver = client.send_transaction(test_blob).await;
        let result = receiver.await.unwrap();
        assert!(result.is_ok());
    }
}

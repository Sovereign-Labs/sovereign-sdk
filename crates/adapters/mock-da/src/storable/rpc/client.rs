use std::time::Duration;

use super::types::*;
use crate::{MockBlock, MockDaSpec, MockDaVerifier};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use sov_rollup_interface::da::{DaSpec, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{DaService, SubmitBlobReceipt};
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize, JsonSchema)]
/// Configuration for [`StorableMockDaClient`].
pub struct MockDaClientConfig {
    ///  Url of the DA server.
    pub url: String,
}

#[derive(Clone)]
/// Http client implementing [`DaService`] for `StorableMockDa`.
pub struct StorableMockDaClient {
    base_url: reqwest::Url,
    client: reqwest::Client,
}

impl StorableMockDaClient {
    /// Creates a new client.
    pub fn new(base_url: &str) -> Result<Self, url::ParseError> {
        let base_url = reqwest::Url::parse(base_url)?;

        Ok(Self {
            base_url,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(5))
                .pool_idle_timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        })
    }

    /// Creates `Self` from MockDaClientConfig.
    pub fn from_config(config: MockDaClientConfig) -> Result<Self, url::ParseError> {
        Self::new(&config.url)
    }

    fn url(&self, path: &str) -> Result<reqwest::Url, url::ParseError> {
        self.base_url.join(path)
    }
}

async fn handle_response<R: DeserializeOwned>(response: reqwest::Response) -> anyhow::Result<R> {
    if !response.status().is_success() {
        let text = response.text().await?;
        tracing::error!("Response: {}", text);
        let error: ErrorResponse = serde_json::from_str(&text)?;
        return Err(anyhow::anyhow!("Server error: {}", error.error));
    }
    let text = response.text().await?;
    tracing::info!("Response: {}", text);

    Ok(serde_json::from_str(&text)?)
}

#[async_trait]
impl DaService for StorableMockDaClient {
    type Spec = MockDaSpec;
    type Config = MockDaClientConfig;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    const GUARANTEES_TRANSACTION_ORDERING: bool = true;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let url = self.url(&format!("/blocks/{height}"))?;
        let response = self.client.get(url).send().await?;
        let block_response: BlockResponse = handle_response(response).await?;

        Ok(block_response.block)
    }

    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let url = self.url(&format!("/block-headers/{height}"))?;
        let response = self.client.get(url).send().await?;

        let header_response: BlockHeaderResponse = handle_response(response).await?;
        Ok(header_response.header)
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let url = self.url("/finalized-block-header")?;
        let response = self.client.get(url).send().await?;

        let header_response: BlockHeaderResponse = handle_response(response).await?;
        Ok(header_response.header)
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let url = self.url("/head-block-header")?;
        let response = self.client.get(url).send().await?;

        let header_response: BlockHeaderResponse = handle_response(response).await?;
        Ok(header_response.header)
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        block.as_relevant_blobs()
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        _blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
        block.get_relevant_proofs()
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();

        let request = SubmitTransactionRequest {
            blob: hex::encode(blob),
        };

        let result = async {
            let url = self.url("/send-transaction")?;
            let response = self.client.post(url).json(&request).send().await?;

            let submit_response: SubmitBlobResponse = handle_response(response).await?;
            Ok(submit_response.receipt)
        }
        .await;

        let _ = tx.send(result);

        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();

        let request = SubmitProofRequest {
            aggregated_proof_data: hex::encode(aggregated_proof_data),
        };

        let result = async {
            let url = self.url("/send-proof")?;
            let response = self.client.post(url).json(&request).send().await?;

            let submit_response: SubmitBlobResponse = handle_response(response).await?;
            Ok(submit_response.receipt)
        }
        .await;

        let _ = tx.send(result);

        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        let url = self.url(&format!("/proofs/{height}"))?;
        let response = self.client.get(url).send().await?;

        let proofs_response: ProofsResponse = handle_response(response).await?;
        let proofs = proofs_response
            .proofs
            .into_iter()
            .map(hex::decode)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(proofs)
    }

    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address {
        let url = self.url("/signer").expect("Bad url");
        let response = self
            .client
            .get(url)
            .send()
            .await
            .expect("Failed to get signer");

        let signer_response: SignerResponse = handle_response(response)
            .await
            .expect("Failed to parse signer response");
        signer_response.address
    }
}

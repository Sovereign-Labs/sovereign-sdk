#![allow(dead_code)]

#[cfg(test)]
mod tests;
mod types;

use crate::celestia::CompactHeader;
pub use crate::twinkle::types::Network;
use crate::twinkle::types::{
    BlobStatus, BlobStatusResponse, HeaderResponse, SubmitBlobAsyncResponse, SubmitBlobRequest,
};
use crate::types::TmHash;
use crate::CelestiaHeader;
use anyhow::Context;
use backon::{ExponentialBuilder, Retryable};
use celestia_types::nmt::Namespace;
use celestia_types::DataAvailabilityHeader;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use tokio::sync::oneshot;

const HEADER_URL: &str = "https://t.tech/v0/header";
const SUBMIT_BLOB_URL: &str = "https://t.tech/v0/blob";
const GET_ALL_BLOBS_URL: &str = "https://t.tech/v0/blob/get_all";
const BLOB_STATUS: &str = "https://t.tech/v0/blob/status";

const AGENT: &str = "sov-celestia-adapter";
const API_KEY_ENV: &str = "SOV_TWINKLE_API_KEY";

// TODO for later:
//  - Get block and header compatible with return types of celestia sender
//  - Logging
//  - Metrics
//  - Unit tests with mockserver
// ---------
//  - Log URLs and timestamps (debug only)

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct TwinkleConfig {
    /// If not set, will be taken from the environment variable ` SOV_TWINKLE_API_KEY `
    api_key: Option<String>,
    network: Network,
    pull_interval_millis: u64,
    /// Timeout for individual HTTP requests in seconds
    /// TODO: Sensible default value
    request_timeout_secs: u64,
    /// Timeout for the entire request including retries in seconds
    /// TODO: Sensible default value
    total_timeout_secs: u64,
    // TODO: Connect timeout: smaller
    // TODO: Pool idle timeout
}

impl TwinkleConfig {
    fn api_key(&self) -> String {
        match &self.api_key {
            None => std::env::var("SOV_TWINKLE_API_KEY").expect(
                "SOV_TWINKLE_API_KEY environment is not set, cannot initialize TwinkleClient",
            ),
            Some(set) => set.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TwinkleClient {
    client: reqwest::Client,
    network: Network,
    // Interval for pulling blob status after submission
    pull_interval: std::time::Duration,
    // Total timeout for a method to complete.
    total_timeout: std::time::Duration,
    backoff_policy: ExponentialBuilder,
}

impl TwinkleClient {
    pub fn from_config(
        config: &TwinkleConfig,
        backoff_policy: ExponentialBuilder,
    ) -> anyhow::Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut auth_value =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.api_key()))?;
        auth_value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static(AGENT),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(config.request_timeout_secs))
            .build()?;

        Ok(Self {
            client,
            network: config.network,
            pull_interval: std::time::Duration::from_millis(config.pull_interval_millis),
            total_timeout: std::time::Duration::from_secs(config.total_timeout_secs),
            backoff_policy,
        })
    }

    async fn blob_status(&self, twinkle_request_id: &str) -> anyhow::Result<BlobStatusResponse> {
        tracing::trace!(%twinkle_request_id, "Checking blob status");

        (|| async {
            let mut request = self.client.get(BLOB_STATUS);
            request = request.query(&[("twinkleRequestId", twinkle_request_id)]);
            let response = request.send().await?;
            decode_on_success(response).await
        })
        .retry(&self.backoff_policy)
        .await
        .with_context(|| format!("Blob status check of request {twinkle_request_id}"))
    }

    async fn submit_blob_to_namespace_and_pull(
        &self,
        blob: String,
        namespace: Namespace,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let timeout = tokio::time::sleep(self.total_timeout);
        tokio::pin!(timeout);

        let start = std::time::Instant::now();
        let namespace = hex::encode(namespace.id_v0().expect("Namespace should be v0"));
        let request = SubmitBlobRequest {
            namespace,
            data: blob,
            asynchronous: true,
            network: self.network.to_string(),
        };

        let submit_start = std::time::Instant::now();
        let submit_response: SubmitBlobAsyncResponse = tokio::select! {
            result = async {
                (|| async {
                    let response = self
                        .client
                        .post(SUBMIT_BLOB_URL)
                        .json(&request)
                        .send()
                        .await?;
                    decode_on_success(response).await
                })
                .retry(&self.backoff_policy)
                .await
            } => result?,
            _ = &mut timeout => {
                return Err(anyhow::anyhow!("Timeout during blob submission after {:?}", self.total_timeout));
            }
        };
        let submit_time = submit_start.elapsed();
        let pull_start = std::time::Instant::now();

        tokio::select! {
            result = async {
                loop {
                    tokio::time::sleep(self.pull_interval).await;
                    // Returning error here, as network errors will be retried inside
                    let response = self
                        .blob_status(&submit_response.twinkle_request_id)
                        .await?;
                    match response.status {
                        BlobStatus::Pending => {
                            continue;
                        }
                        BlobStatus::Included => {
                                      let height = response.height;
                        let receipt = SubmitBlobReceipt::try_from(response)?;
                        tracing::debug!(
                            ?submit_time,
                            pull_time = ?pull_start.elapsed(),
                            total_time = ?start.elapsed(),
                            height = ?height,
                            "Blob has been included");
                        return Ok(receipt);
                        }
                        BlobStatus::Rejected => {
                            tracing::debug!(
                                ?submit_time,
                                pull_time = ?pull_start.elapsed(),
                                total_time = ?start.elapsed(),
                                "Blob has been rejected");
                            anyhow::bail!("Blob has been rejected");
                        }
                    };
                }
            } => result,
            _ = timeout => {
                Err(anyhow::anyhow!("Timeout waiting for blob inclusion after {:?}", self.total_timeout))
            }
        }
    }

    pub async fn submit_blob_to_namespace_inner(
        &self,
        blob: &[u8],
        namespace: Namespace,
    ) -> oneshot::Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        let (tx, rx) = oneshot::channel();
        let client = self.clone();
        let blob = hex::encode(blob);

        tokio::task::spawn(async move {
            tx.send(
                client
                    .submit_blob_to_namespace_and_pull(blob, namespace)
                    .await,
            )
            .expect("Failed to propagate blob submission result into a channel");
        });
        rx
    }

    async fn query_header(&self, height: Option<u64>) -> anyhow::Result<CelestiaHeader> {
        tracing::trace!("Getting head block header");

        let header_response: HeaderResponse = (|| async {
            let mut request = self.client.get(HEADER_URL);
            request = request.query(&[("network", self.network.to_string())]);
            if let Some(height) = height {
                request = request.query(&[("height", height)]);
            }
            let response = request.send().await?;
            decode_on_success(response).await
        })
        .retry(&self.backoff_policy)
        .await
        .context("Head block header")?;

        let compact_header = CompactHeader::from(header_response.header);
        let empty_dah = DataAvailabilityHeader::new_unchecked(Vec::new(), Vec::new());

        let celestia_header = CelestiaHeader::new(empty_dah, compact_header);

        Ok(celestia_header)
    }

    pub async fn get_head_block_header(&self) -> anyhow::Result<CelestiaHeader> {
        self.query_header(None).await
    }

    pub async fn get_block_header_at(&self, height: u64) -> anyhow::Result<CelestiaHeader> {
        self.query_header(Some(height)).await
    }
}

async fn decode_on_success<T: DeserializeOwned>(response: reqwest::Response) -> anyhow::Result<T> {
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await?;
        anyhow::bail!("Failed response to TwinkleAPI: {status:?}: {text}")
    }
    Ok(response.json().await.expect("Failed to decode response"))
}

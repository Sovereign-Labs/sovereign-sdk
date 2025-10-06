#![allow(dead_code)]

use crate::types::TmHash;
use anyhow::Context;
use backon::{ExponentialBuilder, Retryable};
use celestia_types::nmt::Namespace;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use std::fmt::Display;
use tokio::sync::oneshot;

const HEADER_URL: &str = "https://t.tech/v0/header";
const SUBMIT_BLOB_URL: &str = "https://t.tech/v0/blob";
const GET_ALL_BLOBS_URL: &str = "https://t.tech/v0/blob/get_all";
const BLOB_STATUS: &str = "https://t.tech/v0/blob/status";

const AGENT: &str = "sov-celestia-adapter";
const API_KEY_ENV: &str = "SOV_TWINKLE_API_KEY";

// TODO:
//  + Submit async with channel
//  + Config struct: network + env
//  + Retry logic and params
// Other
//  - Get block and header compatible with return types of celestia sender
//  - Logging
//  - Metrics
//  - Unit tests with mockserver

#[derive(Clone, Debug, Copy)]
pub enum Network {
    Mocha,
    Mainnet,
}

impl Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Network::Mocha => "mocha-4".to_string(),
            Network::Mainnet => "mainnet".to_string(),
        };
        write!(f, "{str}")
    }
}

pub struct TwinkleConfig {
    /// If not set, will be taken from the environment variable ` SOV_TWINKLE_API_KEY `
    api_key: Option<String>,
    network: Network,
    pull_interval_millis: u64,
    /// Timeout for individual HTTP requests in seconds
    request_timeout_secs: u64,
    /// Timeout for the entire request including retries in seconds
    total_timeout_secs: u64,
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

#[derive(Debug, Serialize)]
struct SubmitBlobRequest {
    namespace: String,
    data: String,
    asynchronous: bool,
    network: String,
}

#[derive(Debug, Deserialize)]
pub struct SubmitBlobAsyncResponse {
    #[serde(rename = "twinkleRequestId")]
    pub twinkle_request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlobStatus {
    Pending,
    Included,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlobStatusResponse {
    status: BlobStatus,
    #[serde_as(as = "serde_with::base64::Base64")]
    commitment: Vec<u8>,
    // Only set for [`BlobStatus::Included`]
    #[serde(rename = "txId")]
    transaction_id: Option<HexHash>,
    // Only set for [`BlobStatus::Included`]
    height: Option<u64>,
}

impl TryFrom<BlobStatusResponse> for SubmitBlobReceipt<TmHash> {
    type Error = anyhow::Error;

    fn try_from(value: BlobStatusResponse) -> Result<Self, Self::Error> {
        // TODO:
        let blob_hash = value.commitment.try_into().map_err(|e: Vec<u8>| {
            anyhow::anyhow!(
                "Wrong commitment size, should 32 bytes, but was {}",
                e.len(),
            )
        })?;
        let Some(transaction_id) = value.transaction_id else {
            anyhow::bail!("Transaction Id is not present, is status `Included`?");
        };
        Ok(SubmitBlobReceipt {
            blob_hash: HexHash::new(blob_hash),
            da_transaction_id: TmHash(tendermint::Hash::Sha256(transaction_id.0)),
        })
    }
}

#[derive(Clone)]
pub struct TwinkleClient {
    client: reqwest::Client,
    network: Network,
    // Interval for pulling blob status after submission
    pull_interval: std::time::Duration,
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
                    // Returning error here, as network errors will be retried inside
                    let response = self
                        .blob_status(&submit_response.twinkle_request_id)
                        .await?;
                    if matches!(response.status, BlobStatus::Included) {
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
                    tokio::time::sleep(self.pull_interval).await;
                }
            } => result,
            _ = timeout => {
                Err(anyhow::anyhow!("Timeout waiting for blob inclusion after {:?}", self.total_timeout))
            }
        }
    }

    async fn submit_blob_to_namespace(
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
}

async fn decode_on_success<T: DeserializeOwned>(response: reqwest::Response) -> anyhow::Result<T> {
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await?;
        anyhow::bail!("Failed response to TwinkleAPI: {status:?}: {text}")
    }
    Ok(response.json().await.expect("Failed to decode response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_types::nmt::Namespace;

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
    async fn async_blob_submit() -> anyhow::Result<()> {
        sov_test_utils::logging::initialize_or_change_logging_with_filter(
            "debug,hyper=info,sov_celestia_adapter=trace",
        );
        let backoff_policy = ExponentialBuilder::default();
        let twinkle_client = TwinkleClient::from_config(&default_mocha_config(), backoff_policy)?;

        let blob: Vec<u8> = b"hello-from-sov-rust".to_vec();

        let start = std::time::Instant::now();
        let rx = twinkle_client
            .submit_blob_to_namespace(&blob, BATCH_NAMESPACE)
            .await;
        println!("A: {:?}", start.elapsed());
        let res = rx.await?;
        println!("B: {:?}", start.elapsed());
        let receipt = res?;
        println!("RECEIPT: {receipt:?}");

        Ok(())
    }
}

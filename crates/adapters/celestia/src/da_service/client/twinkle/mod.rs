#![allow(dead_code)]
#[cfg(test)]
mod tests;
mod types;

use crate::celestia::CompactHeader;
use crate::config::Network;
use crate::da_service::client::twinkle::types::{
    BlobStatus, BlobStatusResponse, FeePriority, HeaderResponse, SubmitBlobAsyncResponse,
    SubmitBlobRequest, TwinkleNamespaceResponse,
};
use crate::metrics::BlobSubmitMeasurement;
use crate::types::{RollupNamespace, TmHash};
use crate::verifier::address::CelestiaAddress;
use crate::CelestiaHeader;
use anyhow::Context;
use backon::{ExponentialBuilder, Retryable};
use base64::{engine::general_purpose, Engine as _};
use celestia_types::row_namespace_data::NamespaceData;
use serde::de::DeserializeOwned;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use tokio::sync::oneshot;
use tracing::instrument;

const HEADER_URL: &str = "https://t.tech/v0/header";
const SUBMIT_BLOB_URL: &str = "https://t.tech/v0/blob";
const BLOB_STATUS_URL: &str = "https://t.tech/v0/blob/status";
const NAMESPACE_DATA_URL: &str = "https://t.tech/v0/share/get_namespace_data";

// TODO for later:
//  ~ Get block and header compatible with return types of celestia sender
//     + Header: Compact header
//     + Header: DAH
//     ~ Block: Namespace data
//  - Authored blobs
//  ~ Metrics: can be verified with actual rollup
//  - Unit tests with mockserver
//  - More granular retry logic: do not retry on 401, 400. Respect throttling, retry on 500 and timeouts
//  - Config defaults
// ---------

/// Client for [Twinkle](https://t.tech/) service.
/// Documentation: <https://t.tech/docs>.
#[derive(Debug, Clone)]
pub struct TwinkleClient {
    client: reqwest::Client,
    network: Network,
    // Interval for pulling blob status after submission
    pull_interval: std::time::Duration,
    // Total timeout for a method to complete.
    total_timeout: std::time::Duration,
    backoff_policy: ExponentialBuilder,
    fee_priority: Option<FeePriority>,
}

impl TwinkleClient {
    pub(crate) fn new(
        client: reqwest::Client,
        network: Network,
        pull_interval: std::time::Duration,
        total_timeout: std::time::Duration,
        backoff_policy: ExponentialBuilder,
        fee_priority: Option<FeePriority>,
    ) -> Self {
        Self {
            client,
            network,
            pull_interval,
            total_timeout,
            backoff_policy,
            fee_priority,
        }
    }

    #[instrument(skip(self))]
    async fn blob_status(&self, twinkle_request_id: &str) -> anyhow::Result<BlobStatusResponse> {
        tracing::trace!("Checking blob status");
        (|| async {
            let mut request = self.client.get(BLOB_STATUS_URL);
            request = request.query(&[("twinkleRequestId", twinkle_request_id)]);
            let response = request.send().await?;
            decode_on_success(response).await
        })
        .retry(&self.backoff_policy)
        .await
        .with_context(|| format!("Blob status check of request {twinkle_request_id}"))
    }

    #[instrument(skip(self, blob, _signer, namespace), fields(namespace = %namespace.ns_type()))]
    async fn submit_blob_waiting_inclusion(
        &self,
        blob: Vec<u8>,
        namespace: RollupNamespace,
        // TODO: Use it when it becomes supported
        _signer: CelestiaAddress,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let start = std::time::Instant::now();
        let bytes = blob.len();

        let timeout = tokio::time::sleep(self.total_timeout);
        tokio::pin!(timeout);

        tracing::debug!(bytes, "Submitting a blob");

        let request = SubmitBlobRequest {
            namespace: namespace.id(),
            data: blob,
            asynchronous: true,
            network: self.network,
            fee_priority: self.fee_priority,
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
            } => {
                if result.is_err() {
                    let measurement = BlobSubmitMeasurement::new_for_twinkle(
                        namespace.ns_type(),
                        bytes,
                        submit_start.elapsed(),
                        Default::default(),
                        start.elapsed(),
                        None,
                    );
                    sov_metrics::track_metrics(|tracker| {
                        tracker.submit(measurement);
                    });
                }
                result?
            },
            _ = &mut timeout => {
                let measurement = BlobSubmitMeasurement::new_for_twinkle(
                    namespace.ns_type(),
                    bytes,
                    submit_start.elapsed(),
                    Default::default(),
                    start.elapsed(),
                    None,
                );
                sov_metrics::track_metrics(|tracker| {
                    tracker.submit(measurement);
                });
                return Err(anyhow::anyhow!("Timeout during blob submission to namespace={:?} after {:?}", namespace.ns_type(), self.total_timeout));
            }
        };
        let submit_time = submit_start.elapsed();
        let pull_start = std::time::Instant::now();
        tracing::trace!(
            twinkle_request_id = %submit_response.twinkle_request_id,
            "Blob submit request has been accepted by Twinkle, waiting for blob inclusion");

        tokio::select! {
            result = async {
                loop {
                    tokio::time::sleep(self.pull_interval).await;
                    // Returning error here, as network errors will be retried inside
                    let response = self
                        .blob_status(&submit_response.twinkle_request_id)
                        .await?;
                    match &response.status {
                        BlobStatus::Pending => {
                            continue;
                        }
                        BlobStatus::Included {
                            height,
                            ..
                        } => {
                            let height = *height;
                            let receipt = SubmitBlobReceipt::try_from(response)?;
                            tracing::debug!(
                                da_height = %height,
                                tx_hash = %receipt.da_transaction_id,
                                blob_hash = %receipt.blob_hash,
                                bytes,
                                namespace = ?namespace.ns_type(),
                                ?submit_time,
                                pull_time = ?pull_start.elapsed(),
                                total_time = ?start.elapsed(),
                                "Blob has been submitted to Celestia");
                            let measurement = BlobSubmitMeasurement::new_for_twinkle(
                                namespace.ns_type(),
                                bytes,
                                submit_start.elapsed(),
                                pull_start.elapsed(),
                                start.elapsed(),
                                Some(height),
                            );
                            sov_metrics::track_metrics(|tracker| {
                                tracker.submit(measurement);
                            });
                            return Ok(receipt);
                        }
                        BlobStatus::Rejected => {
                            tracing::warn!(
                                ?submit_time,
                                pull_time = ?pull_start.elapsed(),
                                total_time = ?start.elapsed(),
                                "Blob has been rejected");
                            let measurement = BlobSubmitMeasurement::new_for_twinkle(
                                namespace.ns_type(),
                                bytes,
                                submit_start.elapsed(),
                                pull_start.elapsed(),
                                start.elapsed(),
                                None,
                            );
                            sov_metrics::track_metrics(|tracker| {
                                tracker.submit(measurement);
                            });
                            anyhow::bail!("Blob has been rejected");
                        }
                    };
                }
            } => result,
            _ = timeout => {
                let measurement = BlobSubmitMeasurement::new_for_twinkle(
                    namespace.ns_type(),
                    bytes,
                    submit_start.elapsed(),
                    pull_start.elapsed(),
                    start.elapsed(),
                    None,
                );
                sov_metrics::track_metrics(|tracker| {
                    tracker.submit(measurement);
                });
                Err(anyhow::anyhow!("Timeout waiting for blob inclusion after {:?}", self.total_timeout))
            }
        }
    }

    pub async fn submit_blob_to_namespace(
        &self,
        blob: &[u8],
        namespace: RollupNamespace,
        signer: &CelestiaAddress,
    ) -> oneshot::Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        let (tx, rx) = oneshot::channel();
        let client = self.clone();
        let blob = blob.to_vec();
        let signer = signer.clone();

        let _join_handle = tokio::task::spawn(async move {
            tx.send(
                client
                    .submit_blob_waiting_inclusion(blob, namespace, signer)
                    .await,
            )
            .expect("Failed to propagate blob submission result into a channel");
        });
        rx
    }

    async fn query_header(&self, height: Option<u64>) -> anyhow::Result<CelestiaHeader> {
        tracing::trace!(?height, "Getting head block header");

        let header_response: HeaderResponse = (|| async {
            let mut request = self.client.get(HEADER_URL);
            request = request.query(&[("network", self.network)]);
            if let Some(height) = height {
                request = request.query(&[("height", height)]);
            }
            let response = request.send().await?;
            decode_on_success(response).await
        })
        .retry(&self.backoff_policy)
        .await
        .with_context(|| format!("Getting block header at height={height:?}"))?;

        let HeaderResponse { header, dah } = header_response;

        let compact_header = CompactHeader::from(header);
        let dah = dah.try_into()?;
        let celestia_header = CelestiaHeader::new(dah, compact_header);

        Ok(celestia_header)
    }

    pub async fn get_namespace_data(
        &self,
        namespace: RollupNamespace,
        height: u64,
    ) -> anyhow::Result<NamespaceData> {
        let mut request = self.client.get(NAMESPACE_DATA_URL);
        request = request.query(&[("network", self.network)]);
        request = request.query(&[("height", height)]);
        let namespace = general_purpose::STANDARD.encode(namespace.id().as_bytes());
        request = request.query(&[("namespace", namespace)]);
        let response = request.send().await?;
        let twinkle_namespace_data: TwinkleNamespaceResponse = decode_on_success(response).await?;
        twinkle_namespace_data.try_into().map_err(Into::into)
    }

    // Will be used later when necessary data is implemented on Twinkle API
    #[allow(dead_code)]
    #[instrument(skip(self))]
    pub async fn get_head_block_header(&self) -> anyhow::Result<CelestiaHeader> {
        self.query_header(None).await
    }

    #[instrument(skip(self))]
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

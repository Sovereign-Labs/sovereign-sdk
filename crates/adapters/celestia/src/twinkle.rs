#![allow(dead_code)]

use crate::celestia::{CompactHeader, ProtobufHash};
use crate::celestia_tm_version;
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
use tendermint::block::Header as TendermintHeader;
use tendermint::Hash;
use tendermint_proto::Protobuf;
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

#[derive(Clone, Debug, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
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

#[derive(Debug, Clone, Deserialize)]
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

    pub async fn submit_blob_to_namespace(
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

    pub async fn get_head_block_header(&self) -> anyhow::Result<TwinkleBlockHeader> {
        tracing::trace!("Getting head block header");

        let header_response: HeaderResponse = (|| async {
            let mut request = self.client.get(HEADER_URL);
            request = request.query(&[("network", self.network.to_string())]);
            let response = request.send().await?;
            decode_on_success(response).await
        })
        .retry(&self.backoff_policy)
        .await
        .context("Head block header")?;

        Ok(header_response.header)
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

#[derive(Debug, Deserialize)]
pub struct HeaderResponse {
    pub header: TwinkleBlockHeader,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct TwinkleBlockHeader {
    // ~~
    pub version: Version,
    #[serde(rename = "chainId")]
    // +
    pub chain_id: tendermint::chain::Id,
    // +
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub height: tendermint::block::Height,
    //
    pub time: tendermint::Time,
    // TODO: Can it be null/empty string? Should we implement default similar to `block::Id`?
    #[serde(rename = "lastBlockId")]
    pub last_block_id: BlockId,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "lastCommitHash")]
    pub last_commit_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "dataHash")]
    pub data_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "validatorsHash")]
    pub validators_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "nextValidatorsHash")]
    pub next_validators_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "consensusHash")]
    pub consensus_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "appHash")]
    pub app_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "lastResultsHash")]
    pub last_results_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "evidenceHash")]
    pub evidence_hash: tendermint::Hash,
    #[serde(rename = "proposerAddress")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub proposer_address: tendermint::account::Id,
}

impl From<TwinkleBlockHeader> for TendermintHeader {
    fn from(_value: TwinkleBlockHeader) -> Self {
        todo!()
    }
}

impl From<Version> for tendermint::block::header::Version {
    fn from(value: Version) -> Self {
        Self {
            block: value.block,
            app: value.app,
        }
    }
}

impl From<PartSetHeader> for tendermint::block::parts::Header {
    fn from(value: PartSetHeader) -> Self {
        Self::new(value.total, value.hash).expect("Invalid TwinklePartSetHeader")
    }
}

impl From<BlockId> for tendermint::block::Id {
    fn from(value: BlockId) -> Self {
        Self {
            hash: value.hash,
            part_set_header: value.parts.into(),
        }
    }
}

impl From<TwinkleBlockHeader> for CompactHeader {
    fn from(value: TwinkleBlockHeader) -> Self {
        let TwinkleBlockHeader {
            version,
            chain_id,
            height,
            time,
            last_block_id,
            last_commit_hash,
            data_hash,
            validators_hash,
            next_validators_hash,
            consensus_hash,
            app_hash,
            last_results_hash,
            evidence_hash,
            proposer_address,
        } = value;

        let data_hash = match data_hash {
            Hash::Sha256(value) => Some(ProtobufHash(value)),
            Hash::None => None,
        };
        CompactHeader {
            version: Protobuf::<celestia_tm_version::version::Consensus>::encode_vec(
                tendermint::block::header::Version::from(version),
            ),
            chain_id: chain_id.encode_vec(),
            height: height.encode_vec(),
            time: time.encode_vec(),
            last_block_id: Protobuf::<celestia_tm_version::types::BlockId>::encode_vec(
                tendermint::block::Id::from(last_block_id),
            ),
            last_commit_hash: last_commit_hash.encode_vec(),
            data_hash,
            validators_hash: validators_hash.encode_vec(),
            next_validators_hash: next_validators_hash.encode_vec(),
            consensus_hash: consensus_hash.encode_vec(),
            app_hash: app_hash.encode_vec(),
            last_results_hash: last_results_hash.encode_vec(),
            evidence_hash: evidence_hash.encode_vec(),
            proposer_address: proposer_address.encode_vec(),
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Version {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub block: u64,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub app: u64,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlockId {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub hash: tendermint::Hash,
    pub parts: PartSetHeader,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct PartSetHeader {
    pub total: u32,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub hash: tendermint::Hash,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helper::ROLLUP_PROOF_NAMESPACE;
    use crate::verifier::RollupParams;
    use crate::CelestiaConfig;
    use crate::CelestiaService;
    use celestia_types::nmt::Namespace;
    use sov_rollup_interface::node::da::DaService;

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

    #[tokio::test(flavor = "multi_thread")]
    async fn get_head_block_header() -> anyhow::Result<()> {
        let config = CelestiaConfig::dev_config("http://127.0.0.1:26658");
        let params = RollupParams {
            rollup_batch_namespace: BATCH_NAMESPACE,
            rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
        };

        let vanilla_client = CelestiaService::new(config, params).await;

        let backoff_policy = ExponentialBuilder::default();
        let twinkle_client = TwinkleClient::from_config(&default_mocha_config(), backoff_policy)?;

        let twinkle_header = twinkle_client.get_head_block_header().await?;
        println!("Twinkle Header {twinkle_header:?}");
        let height = twinkle_header.height.value();
        let compact_header_twinkle = CompactHeader::from(twinkle_header);

        let vanilla_header = vanilla_client.get_block_header_at(height).await?;

        let compact_header_vanilla = vanilla_header.header;

        assert_eq!(compact_header_twinkle, compact_header_vanilla);

        Ok(())
    }
}

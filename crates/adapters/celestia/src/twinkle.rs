#![allow(dead_code)]

use crate::types::TmHash;
use celestia_types::nmt::Namespace;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::HexString;
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
//  ~ Submit async with channel
//  - Config struct: network + env
// Other
//  - Get block and header compatible with return types of celestia sender
//  - Logging
//  - Retry logic and params

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
    api_key: Option<String>,
    network: Network,
    pull_interval_millis: u64,
    // TODO: Timeouts
}

impl TwinkleConfig {
    fn api_key(&self) -> String {
        match &self.api_key {
            None => {
                
                std::env::var("SOV_TWINKLE_API_KEY").expect(
                    "SOV_TWINKLE_API_KEY environment is not set, cannot initialize TwinkleClient",
                )
            }
            Some(set) => set.clone(),
        }
    }
}

#[derive(Clone)]
pub struct TwinkleClient {
    client: reqwest::Client,
    network: Network,
    pull_interval: std::time::Duration,
}

#[derive(Debug, Deserialize)]
pub struct HeaderResponse {
    pub header: BlockHeader,
}

#[derive(Debug, Deserialize)]
pub struct BlockHeader {
    pub version: Version,
    #[serde(rename = "chainId")]
    pub chain_id: String,
    pub height: String,
    pub time: String,
    #[serde(rename = "lastBlockId")]
    pub last_block_id: BlockId,
    #[serde(rename = "lastCommitHash")]
    pub last_commit_hash: String,
    #[serde(rename = "dataHash")]
    pub data_hash: String,
    #[serde(rename = "validatorsHash")]
    pub validators_hash: String,
    #[serde(rename = "nextValidatorsHash")]
    pub next_validators_hash: String,
    #[serde(rename = "consensusHash")]
    pub consensus_hash: String,
    #[serde(rename = "appHash")]
    pub app_hash: String,
    #[serde(rename = "lastResultsHash")]
    pub last_results_hash: String,
    #[serde(rename = "evidenceHash")]
    pub evidence_hash: String,
    #[serde(rename = "proposerAddress")]
    pub proposer_address: String,
}

#[derive(Debug, Deserialize)]
pub struct Version {
    pub block: String,
    pub app: String,
}

#[derive(Debug, Deserialize)]
pub struct BlockId {
    pub hash: String,
    pub parts: PartSetHeader,
}

#[derive(Debug, Deserialize)]
pub struct PartSetHeader {
    pub total: u32,
    pub hash: String,
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
pub struct SubmitBlobResponse {
    #[serde(rename = "twinkleRequestId")]
    pub twinkle_request_id: String,
    #[serde(rename = "blockHeight")]
    pub block_height: u64,
    #[serde(rename = "celestiaTransactionId")]
    pub celestia_transaction_id: String,
    #[serde(rename = "gasFeeUsdCents")]
    pub gas_fee_usd_cents: f64,
    pub commitment: String,
}

#[derive(Debug, Serialize)]
struct GetAllBlobsRequest {
    height: u64,
    namespaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Blob {
    pub commitment: String,
    pub data: String,
    pub index: u64,
    pub namespace: String,
    #[serde(rename = "shareVersion")]
    pub share_version: u64,
}

#[derive(Debug, Deserialize)]
pub struct BlobStatusResponse {
    status: String,
    // TODO: Other fields
}

impl TwinkleClient {
    pub fn from_config(config: &TwinkleConfig) -> anyhow::Result<Self> {
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
            .build()?;

        Ok(Self {
            client,
            network: config.network,
            pull_interval: std::time::Duration::from_millis(config.pull_interval_millis),
        })
    }

    pub async fn get_header(&self, network: &str, height: u64) -> anyhow::Result<HeaderResponse> {
        let mut request = self.client.get(HEADER_URL);

        request = request.query(&[("network", network)]);
        request = request.query(&[("height", height)]);

        let response = request.send().await?;
        println!("Response {}", response.status());
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await?;
            anyhow::bail!("Failed to query Twinkle: {status:?}: {text}")
        }

        response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Error reading response: {e}"))
    }

    pub async fn submit_blob(
        &self,
        namespace: String,
        data: &[u8],
        network: &str,
    ) -> anyhow::Result<SubmitBlobAsyncResponse> {
        let request = SubmitBlobRequest {
            namespace,
            data: hex::encode(data),
            asynchronous: true,
            network: network.to_string(),
        };

        let response = self
            .client
            .post(SUBMIT_BLOB_URL)
            .json(&request)
            .send()
            .await?;

        println!("Response {}", response.status());
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await?;
            anyhow::bail!("Failed to submit blob Twinkle: {status:?}: {text}")
        }
        response.json().await.map_err(Into::into)
    }

    async fn blob_status(&self, twinkle_request_id: &str) -> anyhow::Result<BlobStatusResponse> {
        let mut request = self.client.get(BLOB_STATUS);
        request = request.query(&[("twinkleRequestId", twinkle_request_id)]);
        let response = request.send().await?;
        println!("Response {}", response.status());
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await?;
            anyhow::bail!("Failed to query blob status: {status:?}: {text}")
        }

        response.json().await.map_err(Into::into)
    }

    async fn submit_blob_to_namespace(
        &self,
        blob: &[u8],
        namespace: Namespace,
    ) -> oneshot::Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        let (tx, rx) = oneshot::channel();

        let namespace = hex::encode(namespace.id_v0().expect("Namespace should be v0"));

        let request = SubmitBlobRequest {
            namespace,
            data: hex::encode(blob),
            asynchronous: true,
            network: "mocha-4".to_string(),
        };

        let response = self
            .client
            .post(SUBMIT_BLOB_URL)
            .json(&request)
            .send()
            // TODO: early submit
            .await
            .unwrap();

        println!("Response {}", response.status());
        let status = response.status();
        if !status.is_success() {
            // TODO: early submit
            let text = response.text().await.unwrap();
            panic!("Failed to submit blob Twinkle: {status:?}: {text}")
        }

        let submit_response: SubmitBlobAsyncResponse = response.json().await.unwrap();

        let client = self.clone();
        tokio::task::spawn(async move {
            let included = "included";
            let mut result = Err(anyhow::anyhow!("Failed to submit in 30"));
            for _ in 0..1_800 {
                if let Ok(response) = client
                    .blob_status(&submit_response.twinkle_request_id)
                    .await {
                    println!("STATUS: {}", response.status);
                    if response.status == included {
                        let r = SubmitBlobReceipt {
                            blob_hash: HexString::new([0u8; 32]),
                            da_transaction_id: TmHash(tendermint::Hash::Sha256([0u8; 32])),
                        };
                        result = Ok(r);
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            tx.send(result).expect("Failed to submit, too bad");
        });
        // TODO: retry
        rx
    }

    pub async fn get_all_blobs(
        &self,
        height: u64,
        namespaces: Vec<String>,
        network: Option<String>,
    ) -> Result<Vec<Blob>, reqwest::Error> {
        let request = GetAllBlobsRequest {
            height,
            namespaces,
            network,
        };

        self.client
            .post(GET_ALL_BLOBS_URL)
            .json(&request)
            .send()
            .await?
            .json()
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // use crate::test_helper::ROLLUP_PROOF_NAMESPACE;
    // use crate::verifier::RollupParams;
    // use crate::{CelestiaConfig, CelestiaService};
    use celestia_types::nmt::Namespace;

    const API_KEY: &str = "TEMP_SECRET";

    fn default_mocha_config() -> TwinkleConfig {
        TwinkleConfig {
            api_key: Some(API_KEY.to_string()),
            network: Network::Mocha,
            pull_interval_millis: 100,
        }
    }

    const BATCH_NAMESPACE: Namespace = Namespace::const_v0(*b"sov-twinkl");

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn get_block_header() -> anyhow::Result<()> {
        let height = 8251465;

        let client = TwinkleClient::from_config(&default_mocha_config())?;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn submit_blob() -> anyhow::Result<()> {
        // let config = CelestiaConfig::dev_config("http: //127.0.0.1:26658");
        // let params = RollupParams {
        //     rollup_batch_namespace: BATCH_NAMESPACE,
        //     rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
        // };
        let batch_namespace = hex::encode(BATCH_NAMESPACE.id_v0().unwrap());

        // let vanilla_client = CelestiaService::new(config, params).await;
        let twinkle_client = TwinkleClient::from_config(&default_mocha_config())?;

        let blob: Vec<u8> = b"hello-from-sov-rust".to_vec();

        let submit_result = twinkle_client
            .submit_blob(batch_namespace, &blob, "mocha-4")
            .await?;

        println!("SUBMIT RESULT: {submit_result:?}");

        // let height = submit_result.block_height;

        // let block_from_vanilla = vanilla_client.get_block_at(height).await?;
        // println!(
        //     "Fetched block from using vanilla client: {}",
        //     block_from_vanilla.header().display()
        // );
        // let relevant_data = vanilla_client.extract_relevant_blobs(&block_from_vanilla);
        //
        // println!("BATCH BLOBS {}", relevant_data.batch_blobs.len());
        // println!("PROOF BLOBS {}", relevant_data.proof_blobs.len());
        // let mut batch_blobs = relevant_data.batch_blobs;
        //
        // assert!(!batch_blobs.is_empty(), "no batch blobs found in {height}");
        //
        // let mut found = false;

        // for vanilla_blob in batch_blobs.iter_mut() {
        //     vanilla_blob.blob.advance(vanilla_blob.total_len());
        //     println!(
        //         "COMPARING VANILLA: {}",
        //         hex::encode(vanilla_blob.blob.accumulator())
        //     );
        //     println!("TWINKLE          : {}", hex::encode(&blob));
        //     if vanilla_blob.blob.accumulator() == &blob[..] {
        //         println!("FOUND: {vanilla_blob:?}");
        //         found = true;
        //         break;
        //     }
        // }
        //
        // assert!(found, "couldn't find submitted blob");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn async_blob_submit() -> anyhow::Result<()> {
        let twinkle_client = TwinkleClient::from_config(&default_mocha_config())?;

        let blob: Vec<u8> = b"hello-from-sov-rust".to_vec();

        let start = std::time::Instant::now();
        let rx = twinkle_client
            .submit_blob_to_namespace(&blob, BATCH_NAMESPACE)
            .await;
        println!("A: {:?}", start.elapsed());
        let res = rx.await?;
        println!("B: {:?}", start.elapsed());
        let receipt = res?;
        println!("RECEIPT: {:?}", receipt);

        Ok(())
    }
}

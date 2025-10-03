#![allow(dead_code)]
use serde::{Deserialize, Serialize};

const HEADER_URL: &str = "https://t.tech/v0/header";
const SUBMIT_BLOB_URL: &str = "https://t.tech/v0/blob";
const GET_ALL_BLOBS_URL: &str = "https://t.tech/v0/blob/get_all";

pub struct TwinkleClient {
    client: reqwest::Client,
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

impl TwinkleClient {
    pub fn new(api_key: &str) -> anyhow::Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut auth_value = reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}"))?;
        auth_value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
        // TODO: Add agent

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self { client })
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
    ) -> anyhow::Result<SubmitBlobResponse> {
        let request = SubmitBlobRequest {
            namespace,
            data: hex::encode(data),
            asynchronous: false,
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
    use crate::test_helper::ROLLUP_PROOF_NAMESPACE;
    use crate::verifier::RollupParams;
    use crate::{CelestiaConfig, CelestiaService};
    use celestia_types::nmt::Namespace;
    use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait};
    use sov_rollup_interface::node::da::{DaService, SlotData};

    const API_KEY: &str = "TEMP_SECRET";

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn get_block_header() -> anyhow::Result<()> {
        let height = 8251465;

        let client = TwinkleClient::new(API_KEY)?;

        let block = client.get_header("mocha-4", height).await?;

        println!("BLOCK HEADER {block:?}");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn submit_blob() -> anyhow::Result<()> {
        let config = CelestiaConfig::dev_config("http://127.0.0.1:26658");
        let namespace = b"sov-twinkl";
        let params = RollupParams {
            rollup_batch_namespace: Namespace::const_v0(*namespace),
            rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
        };
        let batch_namespace = hex::encode(namespace);
        println!(
            "Namespace: {} {}",
            batch_namespace,
            String::from_utf8_lossy(namespace)
        );

        let vanilla_client = CelestiaService::new(config, params).await;
        let twinkle_client = TwinkleClient::new(API_KEY)?;

        let blob: Vec<u8> = b"hello-from-sov-rust".to_vec();

        let submit_result = twinkle_client
            .submit_blob(batch_namespace, &blob, "mocha-4")
            .await?;

        println!("SUBMIT RESULT: {submit_result:?}");

        let height = submit_result.block_height;

        let block_from_vanilla = vanilla_client.get_block_at(height).await?;
        println!(
            "Fetched block from using vanilla client: {}",
            block_from_vanilla.header().display()
        );
        let relevant_data = vanilla_client.extract_relevant_blobs(&block_from_vanilla);

        println!("BATCH BLOBS {}", relevant_data.batch_blobs.len());
        println!("PROOF BLOBS {}", relevant_data.proof_blobs.len());
        let mut batch_blobs = relevant_data.batch_blobs;

        assert!(!batch_blobs.is_empty(), "no batch blobs found in {height}");

        let mut found = false;

        for vanilla_blob in batch_blobs.iter_mut() {
            vanilla_blob.blob.advance(vanilla_blob.total_len());
            println!(
                "COMPARING VANILLA: {}",
                hex::encode(vanilla_blob.blob.accumulator())
            );
            println!("TWINKLE          : {}", hex::encode(&blob));
            if vanilla_blob.blob.accumulator() == &blob[..] {
                println!("FOUND: {vanilla_blob:?}");
                found = true;
                break;
            }
        }

        assert!(found, "couldn't find submitted blob");

        Ok(())
    }
}

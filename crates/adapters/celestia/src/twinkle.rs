#![allow(dead_code)]

use crate::types::TmHash;
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
            None => std::env::var("SOV_TWINKLE_API_KEY").expect(
                "SOV_TWINKLE_API_KEY environment is not set, cannot initialize TwinkleClient",
            ),
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

// #[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlobStatusResponse {
    // Always set,
    status: BlobStatus,
    // #[serde_as(as = "Option<serde_with::base64::Base64>")]
    // commitment: Option<Vec<u8>>,
    commitment: Option<String>,
    #[serde(rename = "txId")]
    // transaction_id: Option<TmHash>,
    transaction_id: Option<String>,
    height: Option<u64>,
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

        decode_on_success(response).await
    }

    async fn blob_status(&self, twinkle_request_id: &str) -> anyhow::Result<BlobStatusResponse> {
        println!("Twinkle request id: {}", twinkle_request_id);
        let mut request = self.client.get(BLOB_STATUS);
        request = request.query(&[("twinkleRequestId", twinkle_request_id)]);
        let response = request.send().await?;
        decode_on_success(response).await
    }

    async fn submit_blob_to_namespace_and_pull(
        &self,
        blob: String,
        namespace: Namespace,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let namespace = hex::encode(namespace.id_v0().expect("Namespace should be v0"));
        let request = SubmitBlobRequest {
            namespace,
            data: blob,
            asynchronous: true,
            network: self.network.to_string(),
        };

        // TODO: Retries
        let response = self
            .client
            .post(SUBMIT_BLOB_URL)
            .json(&request)
            .send()
            .await?;
        let submit_response: SubmitBlobAsyncResponse = decode_on_success(response)
            .await
            .expect("Failed to decode submit lob ");

        for _ in 0..150 {
            let result = self.blob_status(&submit_response.twinkle_request_id).await;
            println!("RESULT: {:?}", result);
            if let Ok(response) = result {
                println!("RESPONSE: {:?}", response);
                if matches!(response.status, BlobStatus::Included) {
                    let r = SubmitBlobReceipt {
                        // blob_hash: HexHash::new(response.commitment.unwrap().try_into().unwrap()),
                        blob_hash: HexHash::new([0u8; 32]),
                        da_transaction_id: TmHash(tendermint::Hash::Sha256([0u8; 32])),
                    };
                    return Ok(r);
                }
            }
            tokio::time::sleep(self.pull_interval).await;
        }
        Err(anyhow::anyhow!("Failed to submit in 30"))
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
            .expect("Failed to send blob result");
        });
        rx
    }
}

async fn decode_on_success<T: DeserializeOwned>(response: reqwest::Response) -> anyhow::Result<T> {
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await?;
        anyhow::bail!("Failed to query blob status: {status:?}: {text}")
    }
    response.json().await.map_err(Into::into)
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
        }
    }

    const BATCH_NAMESPACE: Namespace = Namespace::const_v0(*b"sov-twinkl");

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
        println!("RECEIPT: {receipt:?}");

        Ok(())
    }
}

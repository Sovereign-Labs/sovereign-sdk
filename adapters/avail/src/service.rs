use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

use anyhow::anyhow;
use avail_subxt::AvailConfig;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use subxt::OnlineClient;
use tracing::info;

use crate::avail::{Confidence, ExtrinsicsData};
use crate::spec::block::AvailBlock;
use crate::spec::header::AvailHeader;
use crate::spec::transaction::AvailBlobTransaction;
use crate::spec::DaLayerSpec;

#[derive(Debug, Clone)]
pub struct DaProvider {
    pub node_client: OnlineClient<AvailConfig>,
    pub light_client_url: String,
}

impl DaProvider {
    fn appdata_url(&self, block_num: u64) -> String {
        let light_client_url = self.light_client_url.clone();
        format!("{light_client_url}/v1/appdata/{block_num}")
    }

    fn confidence_url(&self, block_num: u64) -> String {
        let light_client_url = self.light_client_url.clone();
        format!("{light_client_url}/v1/confidence/{block_num}")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RuntimeConfig {
    light_client_url: String,
    #[serde(skip)]
    node_client: Option<OnlineClient<AvailConfig>>,
}

impl PartialEq for RuntimeConfig {
    fn eq(&self, other: &Self) -> bool {
        self.light_client_url == other.light_client_url
    }
}

const POLLING_TIMEOUT: Duration = Duration::from_secs(60);
const POLLING_INTERVAL: Duration = Duration::from_secs(1);

// TODO: Is there a way to avoid coupling to tokio?

async fn wait_for_confidence(confidence_url: &str) -> anyhow::Result<()> {
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() >= POLLING_TIMEOUT {
            return Err(anyhow!("Timeout..."));
        }

        let response = reqwest::get(confidence_url).await?;
        if response.status() != StatusCode::OK {
            info!("Confidence not received");
            tokio::time::sleep(POLLING_INTERVAL).await;
            continue;
        }

        let response: Confidence = serde_json::from_str(&response.text().await?)?;
        if response.confidence < 92.5 {
            info!("Confidence not reached");
            tokio::time::sleep(POLLING_INTERVAL).await;
            continue;
        }

        break;
    }

    Ok(())
}

async fn wait_for_appdata(appdata_url: &str, block: u32) -> anyhow::Result<ExtrinsicsData> {
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() >= POLLING_TIMEOUT {
            return Err(anyhow!("Timeout..."));
        }

        let response = reqwest::get(appdata_url).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(ExtrinsicsData {
                block,
                extrinsics: vec![],
            });
        }
        if response.status() != StatusCode::OK {
            tokio::time::sleep(POLLING_INTERVAL).await;
            continue;
        }

        let appdata: ExtrinsicsData = serde_json::from_str(&response.text().await?)?;
        return Ok(appdata);
    }
}

impl DaService for DaProvider {
    type RuntimeConfig = RuntimeConfig;

    type Spec = DaLayerSpec;

    type FilteredBlock = AvailBlock;

    type Future<T> = Pin<Box<dyn Future<Output = Result<T, Self::Error>> + Send>>;

    type Error = anyhow::Error;

    // Make an RPC call to the node to get the finalized block at the given height, if one exists.
    // If no such block exists, block until one does.
    fn get_finalized_at(&self, height: u64) -> Self::Future<Self::FilteredBlock> {
        let node_client = self.node_client.clone();
        let confidence_url = self.confidence_url(height);
        let appdata_url = self.appdata_url(height);

        Box::pin(async move {
            // NOTE: Only supported case is when application data is present and verified
            wait_for_confidence(&confidence_url).await?;
            let appdata = wait_for_appdata(&appdata_url, height as u32).await?;
            info!("Appdata: {:?}", appdata);

            let hash = node_client
                .rpc()
                .block_hash(Some(height.into()))
                .await?
                .unwrap();

            info!("Hash: {:?}", hash);

            let header = node_client.rpc().header(Some(hash)).await?.unwrap();

            info!("Header: {:?}", header);

            let header = AvailHeader::new(header, hash);
            let transactions = appdata
                .extrinsics
                .iter()
                .map(AvailBlobTransaction::new)
                .collect();
            Ok(AvailBlock {
                header,
                transactions,
            })
        })
    }

    // Make an RPC call to the node to get the block at the given height
    // If no such block exists, block until one does.
    fn get_block_at(&self, height: u64) -> Self::Future<Self::FilteredBlock> {
        self.get_finalized_at(height)
    }

    // Extract the blob transactions relevant to a particular rollup from a block.
    // NOTE: The avail light client is expected to be run in app specific mode, and hence the
    // transactions in the block are already filtered and retrieved by light client.
    fn extract_relevant_txs(
        &self,
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction> {
        block.transactions.clone()
    }

    // Extract the inclusion and completenss proof for filtered block provided.
    // The output of this method will be passed to the verifier.
    // NOTE: The light client here has already completed DA sampling and verification of inclusion and soundness.
    fn get_extraction_proof(
        &self,
        _block: &Self::FilteredBlock,
        _blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        ((), ())
    }

    fn new(
        config: Self::RuntimeConfig,
        _chain_params: <Self::Spec as DaSpec>::ChainParams,
    ) -> Self {
        let node_client = config.node_client.unwrap();
        let light_client_url = config.light_client_url;

        DaProvider {
            node_client,
            light_client_url,
        }
    }

    fn send_transaction(&self, _blob: &[u8]) -> Self::Future<()> {
        unimplemented!("The avail light client does not currently support sending transactions");
    }
}

#[cfg(test)]
mod tests {
    use avail_subxt::build_client;
    use sov_rollup_interface::services::da::DaService;

    use super::DaProvider;
    use crate::service::RuntimeConfig;

    #[tokio::test]
    #[ignore]
    async fn get_finalized_at() {
        tracing_subscriber::fmt::init();

        let node_ws = "ws://127.0.0.1:9944";
        let light_client_url = "http://127.0.0.1:7000".to_string();
        let node_client = Some(build_client(node_ws, false).await.unwrap());
        let runtime_config = RuntimeConfig {
            node_client,
            light_client_url,
        };
        let da_service = DaProvider::new(runtime_config, ());
        da_service.get_finalized_at(1).await.unwrap();
        // panic!();
    }
}

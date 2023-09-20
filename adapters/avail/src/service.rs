use core::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use avail_subxt::api::runtime_types::sp_core::bounded::bounded_vec::BoundedVec;
use avail_subxt::primitives::AvailExtrinsicParams;
use avail_subxt::{api, AvailConfig};
use reqwest::StatusCode;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use sp_core::crypto::Pair as PairTrait;
use sp_keyring::sr25519::sr25519::Pair;
use subxt::tx::PairSigner;
use subxt::OnlineClient;
use tracing::info;

use crate::avail::{Confidence, ExtrinsicsData};
use crate::spec::block::AvailBlock;
use crate::spec::header::AvailHeader;
use crate::spec::transaction::AvailBlobTransaction;
use crate::spec::DaLayerSpec;
use crate::verifier::Verifier;

/// Runtime configuration for the DA service
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct DaServiceConfig {
    pub light_client_url: String,
    pub node_client_url: String,
    //TODO: Safer strategy to load seed so it is not accidentally revealed.
    pub seed: String,
}

#[derive(Clone)]
pub struct DaProvider {
    pub node_client: OnlineClient<AvailConfig>,
    pub light_client_url: String,
    signer: PairSigner<AvailConfig, Pair>,
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

    pub async fn new(config: DaServiceConfig) -> Self {
        let pair = Pair::from_string_with_seed(&config.seed, None).unwrap();
        let signer = PairSigner::<AvailConfig, Pair>::new(pair.0.clone());

        let node_client = avail_subxt::build_client(config.node_client_url.to_string(), false)
            .await
            .unwrap();
        let light_client_url = config.light_client_url;

        DaProvider {
            node_client,
            light_client_url,
            signer,
        }
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

#[async_trait]
impl DaService for DaProvider {
    type Spec = DaLayerSpec;

    type FilteredBlock = AvailBlock;

    type Verifier = Verifier;

    type Error = anyhow::Error;

    // Make an RPC call to the node to get the finalized block at the given height, if one exists.
    // If no such block exists, block until one does.
    async fn get_finalized_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let node_client = self.node_client.clone();
        let confidence_url = self.confidence_url(height);
        let appdata_url = self.appdata_url(height);

        wait_for_confidence(&confidence_url).await?;
        let appdata = wait_for_appdata(&appdata_url, height as u32).await?;
        info!("Appdata: {:?}", appdata);

        let hash = node_client
            .rpc()
            .block_hash(Some(height.into()))
            .await?
            .unwrap();

        let header = node_client.rpc().header(Some(hash)).await?.unwrap();

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
    }

    // Make an RPC call to the node to get the block at the given height
    // If no such block exists, block until one does.
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        self.get_finalized_at(height).await
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
    async fn get_extraction_proof(
        &self,
        _block: &Self::FilteredBlock,
        _blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        ((), ())
    }

    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error> {
        let data_transfer = api::tx()
            .data_availability()
            .submit_data(BoundedVec(blob.to_vec()));

        let extrinsic_params = AvailExtrinsicParams::new_with_app_id(7.into());

        let h = self
            .node_client
            .tx()
            .sign_and_submit_then_watch(&data_transfer, &self.signer, extrinsic_params)
            .await?;

        println!("Transaction submitted: {:#?}", h.extrinsic_hash());

        Ok(())
    }
}

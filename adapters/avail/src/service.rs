use core::time::Duration;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::anyhow;
use async_trait::async_trait;
use avail_subxt::api::runtime_types::sp_core::bounded::bounded_vec::BoundedVec;
use avail_subxt::primitives::AvailExtrinsicParams;
use avail_subxt::{api, AvailConfig};
use pin_project::pin_project;
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
    pub polling_timeout: Option<u64>,
    pub polling_interval: Option<u64>,
    pub app_id: u32,
}

const DEFAULT_POLLING_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_POLLING_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct DaProvider {
    pub node_client: OnlineClient<AvailConfig>,
    pub light_client_url: String,
    signer: PairSigner<AvailConfig, Pair>,
    polling_timeout: Duration,
    polling_interval: Duration,
    app_id: u32,
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
            polling_timeout: match config.polling_timeout {
                Some(i) => Duration::from_secs(i),
                None => DEFAULT_POLLING_TIMEOUT,
            },
            polling_interval: match config.polling_interval {
                Some(i) => Duration::from_secs(i),
                None => DEFAULT_POLLING_INTERVAL,
            },
            app_id: config.app_id,
        }
    }
}

// TODO: Is there a way to avoid coupling to tokio?

async fn wait_for_confidence(
    confidence_url: &str,
    polling_timeout: Duration,
    polling_interval: Duration,
) -> anyhow::Result<()> {
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() >= polling_timeout {
            return Err(anyhow!(
                "Confidence not received after timeout: {}s",
                polling_timeout.as_secs()
            ));
        }

        let response = reqwest::get(confidence_url).await?;
        if response.status() != StatusCode::OK {
            info!("Confidence not received");
            tokio::time::sleep(polling_interval).await;
            continue;
        }

        let response: Confidence = serde_json::from_str(&response.text().await?)?;
        if response.confidence < 92.5 {
            info!("Confidence not reached");
            tokio::time::sleep(polling_interval).await;
            continue;
        }

        break;
    }

    Ok(())
}

async fn wait_for_appdata(
    appdata_url: &str,
    block: u32,
    polling_timeout: Duration,
    polling_interval: Duration,
) -> anyhow::Result<ExtrinsicsData> {
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() >= polling_timeout {
            return Err(anyhow!(
                "RPC call for filtered block to light client timed out. Timeout: {}s",
                polling_timeout.as_secs()
            ));
        }

        let response = reqwest::get(appdata_url).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(ExtrinsicsData {
                block,
                extrinsics: vec![],
            });
        }
        if response.status() != StatusCode::OK {
            tokio::time::sleep(polling_interval).await;
            continue;
        }

        let appdata: ExtrinsicsData = serde_json::from_str(&response.text().await?)?;
        return Ok(appdata);
    }
}

/// Convincice copy from [`subxt::blocks::BlocksClient`]
type BlockStream<T> = Pin<Box<dyn futures::Stream<Item = Result<T, subxt::Error>> + Send>>;
/// Wrapper around return of [`subxt::blocks::BlocksClient::subscribe_finalized`]
/// that maps `Ok` variant to [`AvailHeader`] and `Error` variant to [`anyhow::Error`]
#[pin_project]
pub struct AvailBlockHeaderStream {
    #[pin]
    inner: BlockStream<subxt::blocks::Block<AvailConfig, OnlineClient<AvailConfig>>>,
}

impl futures::Stream for AvailBlockHeaderStream {
    type Item = anyhow::Result<AvailHeader>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let poll_result = this.inner.poll_next(cx);
        Poll::Ready(match poll_result {
            Poll::Ready(Some(Ok(block))) => Some(Ok(AvailHeader::from(block))),
            Poll::Ready(Some(Err(e))) => Some(Err(e.into())),
            Poll::Ready(None) => None,
            Poll::Pending => return Poll::Pending,
        })
    }
}

#[async_trait]
impl DaService for DaProvider {
    type Spec = DaLayerSpec;

    type Verifier = Verifier;

    type FilteredBlock = AvailBlock;
    type HeaderStream = AvailBlockHeaderStream;
    type TransactionId = ();
    type Error = anyhow::Error;

    // Make an RPC call to the node to get the block at the given height, if one exists.
    // If no such block exists, block until one does.
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let node_client = self.node_client.clone();
        let confidence_url = self.confidence_url(height);
        let appdata_url = self.appdata_url(height);

        wait_for_confidence(&confidence_url, self.polling_timeout, self.polling_interval).await?;
        let appdata = wait_for_appdata(
            &appdata_url,
            height as u32,
            self.polling_timeout,
            self.polling_interval,
        )
        .await?;
        info!("Appdata: {:?}", appdata);

        let hash = match { node_client.rpc().block_hash(Some(height.into())).await? } {
            Some(i) => i,
            None => return Err(anyhow!("Hash for height: {} not found.", height)),
        };

        let header = match { node_client.rpc().header(Some(hash)).await? } {
            Some(i) => i,
            None => return Err(anyhow!("Header for hash: {} not found.", hash)),
        };

        let header = AvailHeader::new(header, hash);
        let transactions: Result<Vec<AvailBlobTransaction>, anyhow::Error> = appdata
            .extrinsics
            .iter()
            .map(AvailBlobTransaction::new)
            .collect();

        let transactions = transactions?;
        Ok(AvailBlock {
            header,
            transactions,
        })
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let node_client = self.node_client.clone();
        let finalized_header_hash = node_client.rpc().finalized_head().await?;

        let header = node_client
            .rpc()
            .header(Some(finalized_header_hash))
            .await?
            .ok_or(anyhow::anyhow!("No finalized head found"))?;
        let header = AvailHeader::new(header, finalized_header_hash);
        Ok(header)
    }

    async fn subscribe_finalized_header(&self) -> Result<Self::HeaderStream, Self::Error> {
        let block_stream = self.node_client.blocks().subscribe_finalized().await?;
        Ok(AvailBlockHeaderStream {
            inner: block_stream,
        })
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let node_client = self.node_client.clone();
        let latest_block = node_client.blocks().at_latest().await?;

        Ok(latest_block.into())
    }

    // Extract the blob transactions relevant to a particular rollup from a block.
    // NOTE: The avail light client is expected to be run in app specific mode, and hence the
    // transactions in the block are already filtered and retrieved by light client.
    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction> {
        block.transactions.clone()
    }

    // Extract the inclusion and completeness proof for filtered block provided.
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

        let extrinsic_params = AvailExtrinsicParams::new_with_app_id(self.app_id.into());

        let h = self
            .node_client
            .tx()
            .sign_and_submit_then_watch(&data_transfer, &self.signer, extrinsic_params)
            .await?;

        info!("Transaction submitted: {:#?}", h.extrinsic_hash());

        Ok(())
    }

    async fn send_aggregated_zk_proof(&self, _proof: &[u8]) -> Result<u64, Self::Error> {
        unimplemented!()
    }

    async fn get_aggregated_proofs_at(&self, _height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        unimplemented!()
    }
}

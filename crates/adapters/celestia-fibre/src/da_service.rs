use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use celestia_client::api::fibre::{BlobID, DownloadOptions, FibreClient, FibreClientConfig};
use celestia_client::tx::{SigningKey, TxConfig};
use celestia_client::types::nmt::Namespace;
use celestia_client::types::state::AddressTrait;
use celestia_client::types::Blob;
use sov_celestia_adapter::types::TmHash;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{DaProof, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{DaService, SlotData, SubmitBlobReceipt};
use tokio::sync::oneshot;

use crate::{CelestiaFibreConfig, CelestiaFibreSpec, CelestiaFibreVerifier, FibreBlob};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilteredFibreBlock {
    pub header: CelestiaHeader,
    pub batch_blobs: Vec<FibreBlob>,
    pub proof_blobs: Vec<FibreBlob>,
}

impl SlotData for FilteredFibreBlock {
    type BlockHeader = CelestiaHeader;

    fn hash(&self) -> [u8; 32] {
        SlotData::hash(&self.header)
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn timestamp(&self) -> sov_rollup_interface::da::Time {
        SlotData::timestamp(&self.header)
    }
}

#[derive(Clone)]
pub struct CelestiaFibreDaService {
    client: Arc<celestia_client::Client>,
    grpc: celestia_grpc::GrpcClient,
    fibre: Arc<FibreClient>,
    signing_key: Option<SigningKey>,
    signer_address: Option<CelestiaAddress>,
    rollup_batch_namespace: Namespace,
    rollup_proof_namespace: Namespace,
}

impl CelestiaFibreDaService {
    pub async fn new(
        config: CelestiaFibreConfig,
        chain_params: RollupParams,
    ) -> anyhow::Result<Self> {
        tracing::info!(?config, "Initializing Celestia Fibre Adapter");

        let mut rpc = celestia_client::Client::builder().rpc_url(&config.rpc_url);
        if let Some(token) = &config.rpc_auth_token {
            rpc = rpc.rpc_auth_token(token);
        }
        let client = rpc
            .build()
            .await
            .context("Failed to build celestia-client")?;

        let mut endpoint = celestia_grpc::Endpoint::new(config.grpc_url.clone());
        if let Some(token) = &config.grpc_auth_token {
            endpoint = endpoint.metadata("x-token", token);
        }
        let mut grpc = celestia_grpc::GrpcClient::builder().endpoint(endpoint);
        let mut signing_key = None;
        if let Some(key_hex) = &config.signer_private_key {
            let key_bytes = hex::decode(key_hex).context("`signer_private_key` is not hex")?;
            signing_key = Some(SigningKey::from_slice(&key_bytes).context("Invalid signer key")?);
            grpc = grpc.private_key(&key_bytes);
        }
        let grpc = grpc.build().context("Failed to build gRPC client")?;

        let chain_id = grpc.chain_id().await.context("Failed to fetch chain id")?;
        let fibre = FibreClient::from_grpc_client(
            grpc.clone(),
            FibreClientConfig::new(chain_id.as_str())?,
        )?;

        let signer_address = grpc
            .get_account_address()
            .map(|address| CelestiaAddress::try_from(address.as_bytes()))
            .transpose()?;
        if signer_address.is_none() {
            tracing::info!("CelestiaFibreDaService is read-only and won't be able to submit blobs");
        }

        Ok(Self {
            client: Arc::new(client),
            grpc,
            fibre: Arc::new(fibre),
            signing_key,
            signer_address,
            rollup_batch_namespace: chain_params.rollup_batch_namespace,
            rollup_proof_namespace: chain_params.rollup_proof_namespace,
        })
    }

    async fn get_fibre_blobs(
        &self,
        height: u64,
        namespace: Namespace,
    ) -> anyhow::Result<Vec<FibreBlob>> {
        let blobs = self
            .client
            .blob()
            .get_all(height, &[namespace])
            .await?
            .unwrap_or_default();

        let mut fibre_blobs = Vec::with_capacity(blobs.len());
        // shortcut: downloads run sequentially, parallelize if blocks carry many Fibre blobs.
        for blob in blobs {
            let Some((blob_id, sender)) = fibre_blob_ref(&blob) else {
                tracing::debug!(height, ?namespace, "Skipping non-Fibre blob");
                continue;
            };
            let downloaded = self
                .fibre
                .download(&blob_id, DownloadOptions::default())
                .await
                .with_context(|| format!("Failed to download Fibre blob at height {height}"))?;
            fibre_blobs.push(FibreBlob::new(
                sender,
                blob_id.commitment(),
                downloaded.data().to_vec(),
            ));
        }
        Ok(fibre_blobs)
    }

    async fn submit(
        &self,
        data: &[u8],
        namespace: Namespace,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let (Some(signing_key), Some(account)) =
            (&self.signing_key, self.grpc.get_account_address())
        else {
            anyhow::bail!("Signer must be set for submitting blobs");
        };

        let msg = self
            .fibre
            .upload_and_prepare(signing_key, namespace, data, &account)
            .await?;
        let commitment: [u8; 32] = msg
            .payment_promise
            .as_ref()
            .context("MsgPayForFibre has no payment promise")?
            .commitment
            .as_slice()
            .try_into()
            .context("Fibre commitment is not 32 bytes")?;

        let tx = self.grpc.submit_message(msg, TxConfig::default()).await?;
        tracing::info!(
            da_height = tx.height,
            bytes = data.len(),
            commitment = %HexHash::new(commitment),
            "Blob has been submitted to Celestia Fibre"
        );

        Ok(SubmitBlobReceipt {
            blob_hash: HexHash::new(commitment),
            da_transaction_id: TmHash(tx.hash),
        })
    }
}

/// Extracts the Fibre reference from a v2 blob.
///
/// Anyone can post to the rollup namespace, so blobs that aren't well-formed Fibre
/// references are ignored instead of failing the block.
fn fibre_blob_ref(blob: &Blob) -> Option<(BlobID, CelestiaAddress)> {
    let version = u8::try_from(blob.fibre_blob_version()?).ok()?;
    let blob_id = BlobID::new(version, blob.fibre_commitment()?);
    blob_id.validate().ok()?;
    let sender = CelestiaAddress::try_from(blob.signer.as_ref()?.as_bytes()).ok()?;
    Some((blob_id, sender))
}

#[async_trait]
impl DaService for CelestiaFibreDaService {
    type Spec = CelestiaFibreSpec;
    type Config = CelestiaFibreConfig;
    type Verifier = CelestiaFibreVerifier;
    type FilteredBlock = FilteredFibreBlock;
    type Error = anyhow::Error;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let (header, batch_blobs, proof_blobs) = tokio::try_join!(
            self.get_block_header_at(height),
            self.get_fibre_blobs(height, self.rollup_batch_namespace),
            self.get_fibre_blobs(height, self.rollup_proof_namespace),
        )?;
        Ok(FilteredFibreBlock {
            header,
            batch_blobs,
            proof_blobs,
        })
    }

    async fn get_block_header_at(&self, height: u64) -> Result<CelestiaHeader, Self::Error> {
        Ok(self.client.header().get_by_height(height).await?.into())
    }

    async fn get_last_finalized_block_header(&self) -> Result<CelestiaHeader, Self::Error> {
        // Tendermint has instant finality.
        self.get_head_block_header().await
    }

    async fn get_head_block_header(&self) -> Result<CelestiaHeader, Self::Error> {
        Ok(self.client.header().network_head().await?.into())
    }

    fn extract_relevant_blobs(&self, block: &Self::FilteredBlock) -> RelevantBlobs<FibreBlob> {
        RelevantBlobs {
            proof_blobs: block.proof_blobs.clone(),
            batch_blobs: block.batch_blobs.clone(),
        }
    }

    async fn get_extraction_proof(
        &self,
        _block: &Self::FilteredBlock,
        _blobs: &RelevantBlobs<FibreBlob>,
    ) -> RelevantProofs<(), ()> {
        let empty = || DaProof {
            inclusion_proof: (),
            completeness_proof: (),
        };
        RelevantProofs {
            batch: empty(),
            proof: empty(),
        }
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<Result<SubmitBlobReceipt<TmHash>, Self::Error>> {
        let (tx, rx) = oneshot::channel();
        let res = self
            .submit(blob, self.rollup_batch_namespace)
            .await
            .context("Batch submit");
        // UNWRAP: Not possible because the receiver is in the scope still
        tx.send(res).unwrap();
        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof: &[u8],
    ) -> oneshot::Receiver<Result<SubmitBlobReceipt<TmHash>, Self::Error>> {
        let (tx, rx) = oneshot::channel();
        let res = self
            .submit(aggregated_proof, self.rollup_proof_namespace)
            .await
            .context("Proof submit");
        // UNWRAP: Not possible because the receiver is in the scope still
        tx.send(res).unwrap();
        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        use sov_rollup_interface::da::BlobReaderTrait;

        let mut blobs = self
            .get_fibre_blobs(height, self.rollup_proof_namespace)
            .await?;
        Ok(blobs
            .iter_mut()
            .map(|blob| blob.advance(blob.total_len()).to_vec())
            .collect())
    }

    async fn get_signer(&self) -> Option<CelestiaAddress> {
        self.signer_address
    }

    async fn get_approximate_block_time(&self) -> Duration {
        Duration::from_secs(3)
    }
}

#[cfg(test)]
mod tests {
    use celestia_client::types::blob::RawBlob;

    use super::*;

    fn raw_blob(share_version: u32, data: Vec<u8>) -> RawBlob {
        RawBlob {
            namespace_id: Namespace::new_v0(b"fibre").unwrap().id().to_vec(),
            namespace_version: 0,
            share_version,
            data,
            signer: vec![0xAA; 20],
        }
    }

    #[test]
    fn fibre_blob_ref_reads_v2_reference() {
        let commitment = [0x12; 32];
        let mut data = 0u32.to_be_bytes().to_vec();
        data.extend_from_slice(&commitment);
        let blob = Blob::from_raw(raw_blob(2, data)).unwrap();

        let (blob_id, sender) = fibre_blob_ref(&blob).unwrap();

        assert_eq!(blob_id, BlobID::new(0, commitment));
        assert_eq!(
            sender,
            CelestiaAddress::try_from([0xAA; 20].as_slice()).unwrap()
        );
    }

    #[test]
    fn fibre_blob_ref_skips_unsupported_fibre_version() {
        let mut data = 0x0102u32.to_be_bytes().to_vec();
        data.extend_from_slice(&[0x12; 32]);
        let blob = Blob::from_raw(raw_blob(2, data)).unwrap();

        assert_eq!(fibre_blob_ref(&blob), None);
    }

    #[test]
    fn fibre_blob_ref_skips_regular_blob() {
        let blob = Blob::from_raw(raw_blob(1, vec![0x12; 36])).unwrap();

        assert_eq!(fibre_blob_ref(&blob), None);
    }
}

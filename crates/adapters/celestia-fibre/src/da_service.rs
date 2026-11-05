use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use celestia_client::api::fibre::{BlobID, DownloadOptions, FibreClient, FibreClientConfig};
use celestia_client::tx::{SigningKey, TxConfig};
use celestia_client::types::namespace_data::NamespaceData;
use celestia_client::types::nmt::Namespace;
use celestia_client::types::state::AddressTrait;
use sov_celestia_adapter::types::TmHash;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{
    BlobReaderTrait, BlockHeaderTrait, DaProof, RelevantBlobs, RelevantProofs,
};
use sov_rollup_interface::node::da::{DaService, SlotData, SubmitBlobReceipt};
use tokio::sync::oneshot;

use crate::commitment::prove_rows_prefix;
use crate::namespace::{prove_namespace, verify_namespace_proof, FIBRE_BLOB_VERSION};
use crate::{
    CelestiaFibreConfig, CelestiaFibreSpec, CelestiaFibreVerifier, FibreBlob, FibreNamespaceProof,
    FibreRowsProof,
};

/// Celestia block header, the proofs of the rollup namespaces' sequences, and the Fibre blobs
/// they reference, already downloaded from validators.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilteredFibreBlock {
    pub(crate) header: CelestiaHeader,
    pub(crate) batch_namespace_proof: FibreNamespaceProof,
    pub(crate) batch_blobs: Vec<DownloadedFibreBlob>,
    pub(crate) proof_namespace_proof: FibreNamespaceProof,
    pub(crate) proof_blobs: Vec<DownloadedFibreBlob>,
}

/// Full data of a Fibre blob referenced from the rollup namespace.
///
/// Unlike [`FibreBlob`], which keeps only the read prefix when serialized, this always
/// (de)serializes the whole data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DownloadedFibreBlob {
    sender: CelestiaAddress,
    commitment: [u8; 32],
    data: bytes::Bytes,
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

/// [`DaService`] that reads blob references from a bridge node and blob data from Fibre.
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
    /// Connects to the bridge node and consensus gRPC. Read-only unless `signer_private_key` is set.
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

    /// Proves the sequences of `namespace`, selects the Fibre references among them by verifying
    /// that proof like the verifier does, and downloads the blobs they reference.
    async fn download_fibre_blobs(
        &self,
        header: &CelestiaHeader,
        namespace: Namespace,
        namespace_data: NamespaceData,
    ) -> anyhow::Result<(FibreNamespaceProof, Vec<DownloadedFibreBlob>)> {
        let height = header.height();
        let namespace_proof = prove_namespace(header, namespace, namespace_data)
            .with_context(|| format!("Invalid namespace data at height {height}"))?;
        let refs = verify_namespace_proof(header, namespace, &namespace_proof)
            .with_context(|| format!("Invalid namespace proof at height {height}"))?;

        let mut fibre_blobs = Vec::with_capacity(refs.len());
        // TODO: downloads run sequentially, parallelize if blocks carry many Fibre blobs.
        for fibre_ref in refs {
            let blob_id = BlobID::new(FIBRE_BLOB_VERSION, fibre_ref.commitment);
            let downloaded = self
                .fibre
                .download(&blob_id, DownloadOptions::default())
                .await
                .with_context(|| format!("Failed to download Fibre blob at height {height}"))?;
            fibre_blobs.push(DownloadedFibreBlob {
                sender: fibre_ref.sender,
                commitment: fibre_ref.commitment,
                data: downloaded.into_data(),
            });
        }
        Ok((namespace_proof, fibre_blobs))
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

/// Builds the row proof for the read prefix of each downloaded blob; `read_lens` follows the same
/// order. A blob the node can't prove gets `None` and makes the block unprovable, but must not stop
/// the node: anyone can post such a blob to the rollup namespace.
fn prove_read_prefixes(
    blobs: &[DownloadedFibreBlob],
    read_lens: &[usize],
) -> Vec<Option<FibreRowsProof>> {
    assert_eq!(
        blobs.len(),
        read_lens.len(),
        "Bug: relevant blobs were not extracted from this block"
    );
    blobs
        .iter()
        .zip(read_lens)
        .map(
            |(blob, read_len)| match prove_rows_prefix(&blob.commitment, &blob.data, *read_len) {
                Ok(proof) => Some(proof),
                Err(error) => {
                    tracing::error!(
                        commitment = %HexHash::new(blob.commitment),
                        %error,
                        "Failed to build Fibre row proof, this block can't be proven"
                    );
                    None
                }
            },
        )
        .collect()
}

#[async_trait]
impl DaService for CelestiaFibreDaService {
    type Spec = CelestiaFibreSpec;
    type Config = CelestiaFibreConfig;
    type Verifier = CelestiaFibreVerifier;
    type FilteredBlock = FilteredFibreBlock;
    type Error = anyhow::Error;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let shares = self.client.share();
        let (header, batch_namespace_data, proof_namespace_data) = tokio::try_join!(
            self.get_block_header_at(height),
            async {
                Ok(shares
                    .get_namespace_data(height, self.rollup_batch_namespace)
                    .await?)
            },
            async {
                Ok(shares
                    .get_namespace_data(height, self.rollup_proof_namespace)
                    .await?)
            },
        )?;
        let ((batch_namespace_proof, batch_blobs), (proof_namespace_proof, proof_blobs)) = tokio::try_join!(
            self.download_fibre_blobs(&header, self.rollup_batch_namespace, batch_namespace_data),
            self.download_fibre_blobs(&header, self.rollup_proof_namespace, proof_namespace_data),
        )?;
        Ok(FilteredFibreBlock {
            header,
            batch_namespace_proof,
            batch_blobs,
            proof_namespace_proof,
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
        let to_blobs = |blobs: &[DownloadedFibreBlob]| {
            blobs
                .iter()
                .map(|blob| FibreBlob::new(blob.sender, blob.commitment, blob.data.clone()))
                .collect()
        };
        RelevantBlobs {
            proof_blobs: to_blobs(&block.proof_blobs),
            batch_blobs: to_blobs(&block.batch_blobs),
        }
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<FibreBlob>,
    ) -> RelevantProofs<Vec<Option<FibreRowsProof>>, FibreNamespaceProof> {
        let read_lens = |blobs: &[FibreBlob]| -> Vec<usize> {
            blobs
                .iter()
                .map(|blob| blob.verified_data().len())
                .collect()
        };
        // Cloning only bumps the reference counts of the blob data.
        let batch = (block.batch_blobs.clone(), read_lens(&blobs.batch_blobs));
        let proof = (block.proof_blobs.clone(), read_lens(&blobs.proof_blobs));

        // Re-encoding blobs is CPU-heavy, keep it off the async workers.
        let (batch, proof) = tokio::task::spawn_blocking(move || {
            (
                prove_read_prefixes(&batch.0, &batch.1),
                prove_read_prefixes(&proof.0, &proof.1),
            )
        })
        .await
        .expect("Fibre row proof generation panicked");

        RelevantProofs {
            batch: DaProof {
                inclusion_proof: batch,
                completeness_proof: block.batch_namespace_proof.clone(),
            },
            proof: DaProof {
                inclusion_proof: proof,
                completeness_proof: block.proof_namespace_proof.clone(),
            },
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
        let (header, namespace_data) = tokio::try_join!(self.get_block_header_at(height), async {
            Ok(self
                .client
                .share()
                .get_namespace_data(height, self.rollup_proof_namespace)
                .await?)
        },)?;
        let (_, blobs) = self
            .download_fibre_blobs(&header, self.rollup_proof_namespace, namespace_data)
            .await?;
        Ok(blobs.into_iter().map(|blob| blob.data.to_vec()).collect())
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
    use super::*;

    #[test]
    fn unprovable_blob_gets_no_proof_without_affecting_others() {
        let data = bytes::Bytes::from((0..1_000u32).map(|i| (i * 7) as u8).collect::<Vec<_>>());
        let config = celestia_client::api::fibre::BlobConfig::for_version(0).unwrap();
        let commitment = celestia_client::api::fibre::EncodedFibreBlob::new(&data, config)
            .unwrap()
            .id()
            .commitment();
        let blob = |commitment| DownloadedFibreBlob {
            sender: CelestiaAddress::try_from([0xAA; 20].as_slice()).unwrap(),
            commitment,
            data: data.clone(),
        };
        let blobs = [blob([0x12; 32]), blob(commitment)];

        let proofs = prove_read_prefixes(&blobs, &[0, 0]);

        assert_eq!(
            proofs.iter().map(Option::is_some).collect::<Vec<_>>(),
            [false, true]
        );
    }
}

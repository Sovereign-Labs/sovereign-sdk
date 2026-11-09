use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use celestia_fibre::{BlobID, DownloadOptions, EncodedBlob};
use celestia_types::nmt::Namespace;
use sov_celestia_common::address::CelestiaAddress;
use sov_celestia_common::config::VerifyOnFetchMode;
use sov_celestia_common::header::CelestiaHeader;
use sov_celestia_common::metrics::RollupNamespace;
use sov_celestia_common::node::{
    apply_verify_on_fetch, catch_integrity_panic, run_maybe_retryable_async_fn_with_retries,
    CelestiaNode, MaybeRetryable,
};
use sov_celestia_common::proofs::{build_inclusion_proofs, IncludedBlob};
use sov_celestia_common::shares::NamespaceDataIterator;
use sov_celestia_common::types::{NamespaceBoundaryProof, NamespaceRelevantData, TmHash};
use sov_celestia_common::verifier::RollupParams;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{
    BlobReaderTrait, DaProof, DaSpec, DaVerifier, RelevantBlobs, RelevantProofs,
};
use sov_rollup_interface::node::da::{DaService, SlotData, SubmitBlobReceipt};
use sov_rollup_interface::Bytes;
use sov_shutdown::SecondaryShutdownController;
use tokio::sync::oneshot;

use crate::blob::FibreBlob;
use crate::config::CelestiaFibreConfig;
use crate::encode::{build_rows_proof, encode};
use crate::metrics::{FibreMeasurement, DOWNLOAD, ENCODE, SUBMIT_PAY_FOR_FIBRE};
use crate::proof::FibreBlobProof;
use crate::spec::CelestiaFibreSpec;
use crate::verifier::{
    parse_v2_pointer, supported_share, CelestiaFibreVerifier, FIBRE_SHARE_VERSION,
};

/// Cached Fibre encoding of a downloaded blob; skipped by serde and ignored by equality.
#[derive(Clone, Default)]
struct EncodedCache(OnceLock<Arc<EncodedBlob>>);

impl PartialEq for EncodedCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for EncodedCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().is_some().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DownloadedFibre {
    pub share_idx: usize,
    pub commitment: [u8; 32],
    pub data: Bytes,
    #[serde(skip)]
    encoded: EncodedCache,
}

impl DownloadedFibre {
    pub fn new(share_idx: usize, commitment: [u8; 32], data: Bytes) -> Self {
        Self {
            share_idx,
            commitment,
            data,
            encoded: EncodedCache::default(),
        }
    }

    fn store_encoded(&self, encoded: EncodedBlob) {
        let _ = self.encoded.0.set(Arc::new(encoded));
    }

    async fn encoded_blob(&self) -> anyhow::Result<&EncodedBlob> {
        if self.encoded.0.get().is_none() {
            let encoded = encode(&self.data).await?;
            let _ = self.encoded.0.set(Arc::new(encoded));
        }
        Ok(self
            .encoded
            .0
            .get()
            .expect("Fibre encoding was just stored")
            .as_ref())
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilteredFibreBlock {
    header: CelestiaHeader,
    batch: NamespaceRelevantData,
    proof: NamespaceRelevantData,
    batch_blobs: Vec<DownloadedFibre>,
    proof_blobs: Vec<DownloadedFibre>,
}

impl FilteredFibreBlock {
    pub fn new(
        header: CelestiaHeader,
        batch: NamespaceRelevantData,
        proof: NamespaceRelevantData,
        batch_blobs: Vec<DownloadedFibre>,
        proof_blobs: Vec<DownloadedFibre>,
    ) -> Self {
        Self {
            header,
            batch,
            proof,
            batch_blobs,
            proof_blobs,
        }
    }
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
        use sov_rollup_interface::da::BlockHeaderTrait;
        self.header.time()
    }
}

pub fn extract_relevant_blobs(block: &FilteredFibreBlock) -> RelevantBlobs<FibreBlob> {
    RelevantBlobs {
        batch_blobs: blobs_in_namespace(&block.batch, &block.batch_blobs),
        proof_blobs: blobs_in_namespace(&block.proof, &block.proof_blobs),
    }
}

fn blobs_in_namespace(
    data: &NamespaceRelevantData,
    downloaded: &[DownloadedFibre],
) -> Vec<FibreBlob> {
    let mut output = Vec::new();
    for seq in NamespaceDataIterator::new(&data.data) {
        let share = &seq.shares[0];
        if share.info_byte().map(|info| info.version()) != Some(FIBRE_SHARE_VERSION) {
            continue;
        }
        let pointer =
            parse_v2_pointer(share).expect("v2 pointer was validated when the block was loaded");
        let found = downloaded
            .iter()
            .find(|blob| blob.share_idx == seq.range_in_ns.start)
            .expect("Fibre data missing for pointer");
        assert_eq!(found.commitment, pointer.commitment);
        let sender = share
            .signer()
            .map(CelestiaAddress)
            .expect("Fibre pointer has a signer");
        output.push(FibreBlob::new(
            sender,
            pointer.commitment,
            seq.range_in_ns.clone(),
            found.data.clone(),
        ));
    }
    output
}

pub async fn get_extraction_proof(
    block: &FilteredFibreBlock,
    blobs: &RelevantBlobs<FibreBlob>,
) -> RelevantProofs<Vec<FibreBlobProof>, Option<NamespaceBoundaryProof>> {
    try_get_extraction_proof(block, blobs)
        .await
        .expect("Fibre extraction proof")
}

async fn try_get_extraction_proof(
    block: &FilteredFibreBlock,
    blobs: &RelevantBlobs<FibreBlob>,
) -> anyhow::Result<RelevantProofs<Vec<FibreBlobProof>, Option<NamespaceBoundaryProof>>> {
    Ok(RelevantProofs {
        batch: prove_namespace(
            &block.header,
            &block.batch,
            &blobs.batch_blobs,
            &block.batch_blobs,
        )
        .await?,
        proof: prove_namespace(
            &block.header,
            &block.proof,
            &blobs.proof_blobs,
            &block.proof_blobs,
        )
        .await?,
    })
}

async fn prove_namespace(
    header: &CelestiaHeader,
    data: &NamespaceRelevantData,
    blobs: &[FibreBlob],
    downloaded: &[DownloadedFibre],
) -> anyhow::Result<DaProof<Vec<FibreBlobProof>, Option<NamespaceBoundaryProof>>> {
    let included = blobs
        .iter()
        .map(|blob| {
            let start = blob.range_in_namespace().start;
            IncludedBlob {
                start,
                proven_end: start + 1,
                occupied_end: start + 1,
            }
        })
        .collect::<Vec<_>>();
    let pointers = build_inclusion_proofs(header, data, &included, supported_share);
    let mut fibre_proofs = Vec::with_capacity(pointers.len());
    for pointer in pointers {
        let start = pointer
            .range_proofs
            .first()
            .map(|range| range.start_share_idx);
        let rows = match downloaded
            .iter()
            .find(|stored| Some(stored.share_idx) == start)
        {
            Some(stored) => {
                let read_len = blobs
                    .iter()
                    .find(|blob| Some(blob.range_in_namespace().start) == start)
                    .map(|blob| blob.verified_data().len())
                    .unwrap_or(0);
                Some(build_rows_proof(
                    stored.encoded_blob().await?,
                    &stored.data,
                    read_len,
                )?)
            }
            None => None,
        };
        fibre_proofs.push(FibreBlobProof { pointer, rows });
    }
    Ok(DaProof {
        inclusion_proof: fibre_proofs,
        completeness_proof: NamespaceBoundaryProof::from_namespace_data(data),
    })
}

#[derive(Clone)]
pub struct CelestiaFibreService {
    node: CelestiaNode,
    params: RollupParams,
    signing_key: Option<celestia_client::tx::SigningKey>,
    download_timeout: Duration,
    safe_lead_time: Duration,
    verify_on_fetch: VerifyOnFetchMode,
}

impl CelestiaFibreService {
    pub async fn new(
        config: CelestiaFibreConfig,
        params: RollupParams,
        _shutdown: &SecondaryShutdownController,
    ) -> anyhow::Result<Self> {
        let signing_key = config.signing_key()?;
        let (fibre_url, fibre_token) = config
            .fibre_grpc_endpoint()
            .ok_or_else(|| anyhow::anyhow!("Fibre gRPC endpoint is not configured"))?;
        let endpoints = endpoints(&config);
        let probe = sov_celestia_common::config::build_client(endpoints, |builder| builder).await?;
        let chain_id = probe.chain_id().to_string();
        drop(probe);
        let mut fibre_endpoint = celestia_fibre::Endpoint::new(fibre_url);
        if let Some(token) = fibre_token {
            fibre_endpoint = fibre_endpoint.metadata("x-token", token);
        }
        let fibre = celestia_fibre::FibreClient::from_endpoint(
            fibre_endpoint,
            celestia_fibre::FibreClientConfig::new(chain_id)?,
        )?;
        let client = sov_celestia_common::config::build_client(endpoints, |builder| {
            builder.fibre_client(fibre)
        })
        .await?;
        Ok(Self {
            node: CelestiaNode::new(
                client,
                Duration::from_secs(config.request_timeout_secs.get()),
                config.backoff(),
            ),
            params,
            signing_key,
            download_timeout: Duration::from_secs(config.download_timeout_secs.get()),
            safe_lead_time: Duration::from_millis(config.safe_lead_time_ms),
            verify_on_fetch: config.verify_on_fetch,
        })
    }
}

fn endpoints(config: &CelestiaFibreConfig) -> sov_celestia_common::config::ClientEndpoints<'_> {
    sov_celestia_common::config::ClientEndpoints {
        rpc_url: &config.rpc_url,
        rpc_auth_token: config.rpc_auth_token.as_deref(),
        rpc_fallback_endpoints: &config.rpc_fallback_endpoints,
        rpc_health_check_interval_secs: config.rpc_health_check_interval_secs,
        rpc_max_head_age_secs: config.rpc_max_head_age_secs,
        grpc_url: config.grpc_url.as_deref(),
        grpc_auth_token: config.grpc_auth_token.as_deref(),
        grpc_fallback_endpoints: &config.grpc_fallback_endpoints,
        signer_private_key: config.signer_private_key.as_deref(),
        api_request_timeout: Duration::from_secs(config.api_request_timeout_secs.get()),
    }
}

#[async_trait]
impl DaService for CelestiaFibreService {
    type Spec = CelestiaFibreSpec;
    type Config = CelestiaFibreConfig;
    type Verifier = CelestiaFibreVerifier;
    type FilteredBlock = FilteredFibreBlock;
    type Error = anyhow::Error;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let (header, batch_rows, proof_rows) = self
            .node
            .fetch_block_parts(
                height,
                self.params.rollup_batch_namespace,
                self.params.rollup_proof_namespace,
            )
            .await?;
        let batch = NamespaceRelevantData::new(self.params.rollup_batch_namespace, batch_rows);
        let proof = NamespaceRelevantData::new(self.params.rollup_proof_namespace, proof_rows);
        let batch_blobs = self.download_namespace(&batch, height).await?;
        let proof_blobs = self.download_namespace(&proof, height).await?;
        let block = FilteredFibreBlock::new(header, batch, proof, batch_blobs, proof_blobs);
        match self.verify_on_fetch {
            VerifyOnFetchMode::Off => {}
            mode => {
                let verified = self.verify_block_integrity(&block).await;
                apply_verify_on_fetch(mode, height, || verified)?;
            }
        }
        Ok(block)
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.get_head_block_header().await
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        run_maybe_retryable_async_fn_with_retries(
            self.node.backoff,
            || self.node.get_head_header(),
            "get_head_block_header",
        )
        .await
    }

    fn safe_lead_time(&self) -> Duration {
        self.safe_lead_time
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        extract_relevant_blobs(block)
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
        get_extraction_proof(block, blobs).await
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        self.submit(blob, self.params.rollup_batch_namespace).await
    }

    async fn send_proof(
        &self,
        aggregated_proof: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        self.submit(aggregated_proof, self.params.rollup_proof_namespace)
            .await
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        let proof_rows = run_maybe_retryable_async_fn_with_retries(
            self.node.backoff,
            || {
                self.node.get_namespace_data_at(
                    height,
                    self.params.rollup_proof_namespace,
                    RollupNamespace::Proof,
                )
            },
            "get_proofs_at",
        )
        .await?;
        let proof = NamespaceRelevantData::new(self.params.rollup_proof_namespace, proof_rows);
        let downloaded = self.download_namespace(&proof, height).await?;
        Ok(downloaded
            .into_iter()
            .map(|blob| blob.data.to_vec())
            .collect())
    }

    async fn get_signer(&self) -> Option<<Self::Spec as DaSpec>::Address> {
        self.node.client.address().ok().map(CelestiaAddress)
    }

    async fn get_approximate_block_time(&self) -> Duration {
        Duration::from_secs(3)
    }
}

impl CelestiaFibreService {
    async fn download_namespace(
        &self,
        data: &NamespaceRelevantData,
        height: u64,
    ) -> anyhow::Result<Vec<DownloadedFibre>> {
        let mut downloaded = Vec::new();
        for seq in NamespaceDataIterator::new(&data.data) {
            let share = &seq.shares[0];
            if share.info_byte().map(|info| info.version()) != Some(FIBRE_SHARE_VERSION) {
                continue;
            }
            let pointer = parse_v2_pointer(share)?;
            let commitment = pointer.commitment;
            let timeout = self.download_timeout;
            let client = self.node.client.clone();
            let bytes = run_maybe_retryable_async_fn_with_retries(
                self.node.backoff,
                || {
                    let client = client.clone();
                    async move {
                        let started = std::time::Instant::now();
                        let result = tokio::time::timeout(timeout, async {
                            let api = client.fibre().ok_or_else(|| {
                                anyhow::anyhow!("Celestia client has no Fibre endpoint")
                            })?;
                            api.download(
                                &BlobID::new(0, commitment),
                                DownloadOptions {
                                    height: Some(height),
                                    ..DownloadOptions::default()
                                },
                            )
                            .await
                            .map_err(|error| anyhow::anyhow!(error))
                        })
                        .await;
                        let success = matches!(result, Ok(Ok(_)));
                        sov_metrics::track_metrics(|tracker| {
                            tracker.submit(FibreMeasurement {
                                name: DOWNLOAD,
                                bytes: result
                                    .as_ref()
                                    .ok()
                                    .and_then(|r| r.as_ref().ok())
                                    .map(|b| b.data().len())
                                    .unwrap_or(0),
                                elapsed: started.elapsed(),
                                success: Some(success),
                            });
                        });
                        match result {
                            Ok(Ok(blob)) => Ok(Bytes::copy_from_slice(blob.data())),
                            Ok(Err(error)) => Err(MaybeRetryable::Transient(error)),
                            Err(_) => Err(MaybeRetryable::Transient(anyhow::anyhow!(
                                "Fibre download timed out"
                            ))),
                        }
                    }
                },
                "fibre_download",
            )
            .await?;
            let started = std::time::Instant::now();
            let encoded = encode(&bytes).await?;
            sov_metrics::track_metrics(|tracker| {
                tracker.submit(FibreMeasurement {
                    name: ENCODE,
                    bytes: bytes.len(),
                    elapsed: started.elapsed(),
                    success: None,
                });
            });
            anyhow::ensure!(
                encoded.id().commitment() == commitment,
                "downloaded Fibre blob does not match the pointer commitment"
            );
            let stored = DownloadedFibre::new(seq.range_in_ns.start, commitment, bytes);
            stored.store_encoded(encoded);
            downloaded.push(stored);
        }
        Ok(downloaded)
    }

    async fn verify_block_integrity(&self, block: &FilteredFibreBlock) -> anyhow::Result<()> {
        let blobs = catch_integrity_panic("Fibre", || {
            let mut blobs = extract_relevant_blobs(block);
            for blob in blobs
                .batch_blobs
                .iter_mut()
                .chain(blobs.proof_blobs.iter_mut())
            {
                blob.advance(blob.total_len());
            }
            Ok(blobs)
        })?;
        let proofs = try_get_extraction_proof(block, &blobs).await?;
        catch_integrity_panic("Fibre", || {
            CelestiaFibreVerifier::new(self.params)
                .verify_relevant_tx_list(block.header(), &blobs, proofs)
                .map_err(|error| anyhow::anyhow!(error))
        })
    }

    async fn submit(
        &self,
        data: &[u8],
        namespace: Namespace,
    ) -> oneshot::Receiver<Result<SubmitBlobReceipt<TmHash>, anyhow::Error>> {
        let (tx, rx) = oneshot::channel();
        let result = async {
            let Some(key) = &self.signing_key else {
                anyhow::bail!("Signer must be set for submitting Fibre blobs");
            };
            let started = std::time::Instant::now();
            let api = self
                .node
                .client
                .fibre()
                .ok_or_else(|| anyhow::anyhow!("Celestia client has no Fibre endpoint"))?;
            let submitted =
                tokio::time::timeout(self.node.request_timeout, api.put(key, namespace, data))
                    .await
                    .map_err(|_| anyhow::anyhow!("Fibre submit timed out"))?
                    .map_err(|error| anyhow::anyhow!(error))?;
            let info = submitted
                .confirm()
                .await
                .map_err(|error| anyhow::anyhow!(error))?;
            // `FibreApi::put` returns only the tx hash. The commitment is not on that result.
            let encoded = encode(data).await?;
            sov_metrics::track_metrics(|tracker| {
                tracker.submit(FibreMeasurement {
                    name: SUBMIT_PAY_FOR_FIBRE,
                    bytes: data.len(),
                    elapsed: started.elapsed(),
                    success: Some(true),
                });
            });
            Ok(SubmitBlobReceipt {
                blob_hash: HexHash::new(encoded.id().commitment()),
                da_transaction_id: TmHash(info.hash),
            })
        }
        .await;
        let _ = tx.send(result);
        rx
    }
}

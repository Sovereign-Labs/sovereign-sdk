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
    apply_verify_on_fetch, run_maybe_retryable_async_fn_with_retries, CelestiaNode, MaybeRetryable,
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
use crate::metrics::{
    FibreDownloadMeasurement, FibreEncodeMeasurement, SubmitPayForFibreMeasurement,
};
use crate::proof::FibreBlobProof;
use crate::spec::CelestiaFibreSpec;
use crate::verifier::{
    parse_v2_pointer, supported_share, CelestiaFibreVerifier, FIBRE_SHARE_VERSION,
};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct DownloadedFibre {
    pub share_idx: usize,
    pub commitment: [u8; 32],
    pub data: Bytes,
    #[serde(skip)]
    encoded: OnceLock<Arc<EncodedBlob>>,
}

impl DownloadedFibre {
    pub fn new(share_idx: usize, commitment: [u8; 32], data: Bytes) -> Self {
        Self {
            share_idx,
            commitment,
            data,
            encoded: OnceLock::new(),
        }
    }

    fn store_encoded(&self, encoded: EncodedBlob) {
        let _ = self.encoded.set(Arc::new(encoded));
    }

    async fn encoded_blob(&self) -> anyhow::Result<&EncodedBlob> {
        if self.encoded.get().is_none() {
            let encoded = encode(&self.data).await?;
            let _ = self.encoded.set(Arc::new(encoded));
        }
        Ok(self
            .encoded
            .get()
            .expect("Fibre encoding was just stored")
            .as_ref())
    }
}

impl Clone for DownloadedFibre {
    fn clone(&self) -> Self {
        let encoded = OnceLock::new();
        if let Some(cached) = self.encoded.get() {
            let _ = encoded.set(Arc::clone(cached));
        }
        Self {
            share_idx: self.share_idx,
            commitment: self.commitment,
            data: self.data.clone(),
            encoded,
        }
    }
}

impl PartialEq for DownloadedFibre {
    fn eq(&self, other: &Self) -> bool {
        self.share_idx == other.share_idx
            && self.commitment == other.commitment
            && self.data == other.data
    }
}

impl std::fmt::Debug for DownloadedFibre {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadedFibre")
            .field("share_idx", &self.share_idx)
            .field("commitment", &self.commitment)
            .field("data", &self.data)
            .field("encoded", &self.encoded.get().is_some())
            .finish()
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
        let probe =
            sov_celestia_common::config::build_client(endpoints(&config), |builder| builder)
                .await?;
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
        let client = sov_celestia_common::config::build_client(endpoints(&config), |builder| {
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
                            tracker.submit(FibreDownloadMeasurement {
                                bytes: result
                                    .as_ref()
                                    .ok()
                                    .and_then(|r| r.as_ref().ok())
                                    .map(|b| b.data().len())
                                    .unwrap_or(0),
                                elapsed: started.elapsed(),
                                success,
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
                tracker.submit(FibreEncodeMeasurement {
                    bytes: bytes.len(),
                    elapsed: started.elapsed(),
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
        let blobs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut blobs = extract_relevant_blobs(block);
            for blob in blobs
                .batch_blobs
                .iter_mut()
                .chain(blobs.proof_blobs.iter_mut())
            {
                blob.advance(blob.total_len());
            }
            blobs
        }))
        .map_err(integrity_panic)?;
        let proofs = try_get_extraction_proof(block, &blobs).await?;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            CelestiaFibreVerifier::new(self.params)
                .verify_relevant_tx_list(block.header(), &blobs, proofs)
                .map_err(|error| anyhow::anyhow!(error))
        }))
        .map_err(integrity_panic)?
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
                tracker.submit(SubmitPayForFibreMeasurement {
                    bytes: data.len(),
                    elapsed: started.elapsed(),
                    success: true,
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

fn integrity_panic(panic: Box<dyn std::any::Any + Send>) -> anyhow::Error {
    anyhow::anyhow!(
        "Fibre block integrity verification panicked: {}",
        panic_payload_to_string(panic.as_ref())
    )
}

fn panic_payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use celestia_types::consts::appconsts::SHARE_SIZE;
    use celestia_types::{Blob, DataAvailabilityHeader, ExtendedDataSquare, Share};
    use sov_celestia_common::fixtures::{load_block_fixture, BlockFixture};
    use sov_celestia_common::header::ProtobufHash;
    use sov_celestia_common::shares::shares_needed_for_bytes_with_signer;
    use sov_celestia_common::types::NamespaceData;
    use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaVerifier};

    use super::*;

    fn tail_padding() -> Share {
        let mut raw = vec![0u8; SHARE_SIZE];
        let ns = Namespace::TAIL_PADDING.as_bytes();
        raw[..ns.len()].copy_from_slice(ns);
        raw[ns.len()] = 1;
        Share::from_raw(&raw).unwrap()
    }

    fn mocha_params() -> RollupParams {
        RollupParams {
            rollup_batch_namespace: Namespace::const_v0(*b"sov-niko-a"),
            rollup_proof_namespace: Namespace::const_v0([1; 10]),
        }
    }

    fn load_fixture(path: &str) -> BlockFixture {
        let params = mocha_params();
        load_block_fixture(
            params.rollup_batch_namespace,
            params.rollup_proof_namespace,
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path),
        )
        .unwrap()
    }

    fn patch_commitment(share: &Share, commitment: &[u8; 32]) -> Share {
        let mut raw = share.to_vec();
        let payload_start = SHARE_SIZE - share.payload().unwrap().len();
        raw[payload_start..payload_start + 4].copy_from_slice(&0u32.to_be_bytes());
        raw[payload_start + 4..payload_start + 36].copy_from_slice(commitment);
        Share::from_raw(&raw).unwrap()
    }

    fn block_around(
        template: &CelestiaHeader,
        shares: Vec<Share>,
        batch_blobs: Vec<DownloadedFibre>,
        params: &RollupParams,
    ) -> FilteredFibreBlock {
        let mut width = 2usize;
        while width * width < shares.len() {
            width *= 2;
        }
        let mut ods = shares.iter().map(Share::to_vec).collect::<Vec<_>>();
        ods.resize(width * width, tail_padding().to_vec());
        let eds = ExtendedDataSquare::from_ods(ods).unwrap();
        let dah = DataAvailabilityHeader::from_eds(&eds);
        let mut compact = template.header.clone();
        compact.data_hash = Some(ProtobufHash(dah.hash().as_ref().try_into().unwrap()));
        let header = CelestiaHeader::new(dah.clone(), compact);
        let batch_rows = eds
            .get_namespace_data(params.rollup_batch_namespace, &dah, header.height())
            .unwrap()
            .into_iter()
            .map(|(_, row)| row)
            .collect();
        FilteredFibreBlock::new(
            header,
            NamespaceRelevantData::new(
                params.rollup_batch_namespace,
                NamespaceData::new(batch_rows),
            ),
            NamespaceRelevantData::new(params.rollup_proof_namespace, NamespaceData::new(vec![])),
            batch_blobs,
            vec![],
        )
    }

    async fn fibre_block(with_v1: bool) -> (FilteredFibreBlock, Vec<u8>, RollupParams) {
        let params = mocha_params();
        let template = load_fixture("test_data/block_mocha_fibre_v2_1194000");
        let pointer = &template.batch_rows.rows()[0].shares[0];
        let signer = pointer.signer().unwrap();
        let data: Vec<u8> = (0..4_000).map(|i| (i * 13) as u8).collect();
        let commitment = encode(&data).await.unwrap().id().commitment();

        let mut shares = Vec::new();
        if with_v1 {
            shares.extend(
                Blob::new(params.rollup_batch_namespace, vec![7u8; 800], Some(signer))
                    .unwrap()
                    .to_shares()
                    .unwrap(),
            );
        }
        let share_idx = shares.len();
        shares.push(patch_commitment(pointer, &commitment));
        let block = block_around(
            &template.header,
            shares,
            vec![DownloadedFibre::new(
                share_idx,
                commitment,
                Bytes::from(data.clone()),
            )],
            &params,
        );
        (block, data, params)
    }

    async fn verify_supplied(
        block: &FilteredFibreBlock,
        params: RollupParams,
        read_len: usize,
    ) -> RelevantBlobs<FibreBlob> {
        let mut blobs = extract_relevant_blobs(block);
        if read_len > 0 {
            blobs.batch_blobs[0].advance(read_len);
        }
        let proofs = get_extraction_proof(block, &blobs).await;
        CelestiaFibreVerifier::new(params)
            .verify_relevant_tx_list(block.header(), &blobs, proofs)
            .unwrap();
        blobs
    }

    #[tokio::test]
    async fn fibre_round_trip_verifies_a_partial_read_and_skips_v1() {
        let (block, data, params) = fibre_block(true).await;
        let mut blobs = extract_relevant_blobs(&block);
        assert_eq!(blobs.batch_blobs.len(), 1);
        blobs.batch_blobs[0].advance(1000);
        assert_eq!(blobs.batch_blobs[0].verified_data(), &data[..1000]);
        let proofs = get_extraction_proof(&block, &blobs).await;
        assert!(proofs.batch.inclusion_proof.len() > 1);
        CelestiaFibreVerifier::new(params)
            .verify_relevant_tx_list(block.header(), &blobs, proofs)
            .unwrap();
    }

    #[tokio::test]
    async fn mocha_v2_pointer_extracts_matching_data_and_verifies() {
        let params = mocha_params();
        let fixture = load_fixture("test_data/block_mocha_fibre_v2_1194000");
        let pointer_share = &fixture.batch_rows.rows()[0].shares[0];
        let pointer = parse_v2_pointer(pointer_share).unwrap();
        let sender = pointer_share.signer().unwrap();
        assert_eq!(fixture.header.height(), 1_194_000);
        assert_ne!(pointer.commitment, [0; 32]);

        let data = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];
        let commitment = encode(&data).await.unwrap().id().commitment();
        let block = block_around(
            &fixture.header,
            vec![patch_commitment(pointer_share, &commitment)],
            vec![DownloadedFibre::new(
                0,
                commitment,
                Bytes::from(data.clone()),
            )],
            &params,
        );
        let blobs = verify_supplied(&block, params, data.len()).await;
        assert_eq!(blobs.batch_blobs.len(), 1);
        assert!(blobs.proof_blobs.is_empty());
        assert_eq!(blobs.batch_blobs[0].commitment(), commitment);
        assert_eq!(blobs.batch_blobs[0].sender(), CelestiaAddress(sender));
        assert_eq!(blobs.batch_blobs[0].verified_data(), data.as_slice());
    }

    #[tokio::test]
    async fn mocha_mixed_v1_blob_is_proven_skipped() {
        let params = mocha_params();
        let fixture = load_fixture("test_data/block_mocha_mixed_v1_v2_1193993");
        assert_eq!(fixture.header.height(), 1_193_993);
        let mut shares = fixture
            .batch_rows
            .rows()
            .iter()
            .flat_map(|row| row.shares.clone())
            .collect::<Vec<_>>();
        let v1 = &shares[0];
        assert_eq!(v1.info_byte().unwrap().version(), 1);
        let v1_len = v1.sequence_length().unwrap() as usize;
        let v1_shares = shares_needed_for_bytes_with_signer(v1_len, v1.signer().is_some());
        assert_eq!(shares.len(), v1_shares + 1);
        let pointer_share = shares.pop().unwrap();
        let pointer = parse_v2_pointer(&pointer_share).unwrap();
        let sender = pointer_share.signer().unwrap();
        assert_ne!(pointer.commitment, [0; 32]);

        let data = b"mixed-fixture-fibre".to_vec();
        let commitment = encode(&data).await.unwrap().id().commitment();
        shares.push(patch_commitment(&pointer_share, &commitment));
        let share_idx = shares.len() - 1;
        let block = block_around(
            &fixture.header,
            shares,
            vec![DownloadedFibre::new(
                share_idx,
                commitment,
                Bytes::from(data.clone()),
            )],
            &params,
        );
        let mut blobs = extract_relevant_blobs(&block);
        assert_eq!(blobs.batch_blobs.len(), 1);
        assert_eq!(blobs.batch_blobs[0].commitment(), commitment);
        assert_eq!(blobs.batch_blobs[0].sender(), CelestiaAddress(sender));
        blobs.batch_blobs[0].advance(data.len());
        let proofs = get_extraction_proof(&block, &blobs).await;
        assert_eq!(proofs.batch.inclusion_proof.len(), 2);
        assert!(proofs.batch.inclusion_proof[0].rows.is_none());
        assert!(proofs.batch.inclusion_proof[1].rows.is_some());
        CelestiaFibreVerifier::new(params)
            .verify_relevant_tx_list(block.header(), &blobs, proofs)
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a celestia-app >= v10 image and a funded Fibre escrow for the signer"]
    async fn test_submit_blob_through_fibre_correct() -> anyhow::Result<()> {
        let config: CelestiaFibreConfig = serde_json::from_str("{}")?;
        anyhow::ensure!(
            !config.rpc_url.is_empty()
                && config.grpc_url.is_some()
                && config.signing_key()?.is_some(),
            "set SOV_CELESTIA_RPC_URL, SOV_CELESTIA_GRPC_URL and SOV_CELESTIA_SIGNER_KEY"
        );
        let params = RollupParams {
            rollup_batch_namespace: Namespace::const_v0(*b"\0\0sov-test"),
            rollup_proof_namespace: Namespace::const_v0(*b"sov-test-p"),
        };
        let shutdown = SecondaryShutdownController::new();
        let service = CelestiaFibreService::new(config, params, &shutdown).await?;
        let signer = service.get_signer().await.expect("signer");
        let blob = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];
        let height_before = service.get_head_block_header().await?.height();
        let response = service.send_transaction(&blob).await.await??;

        let mut matched = false;
        for _ in 0..40 {
            let head = service.get_head_block_header().await?.height();
            for height in height_before..=head {
                let block = service.get_block_at(height).await?;
                let mut blobs = service.extract_relevant_blobs(&block);
                assert!(
                    blobs
                        .proof_blobs
                        .iter()
                        .all(|proof| proof.commitment() != response.blob_hash.0),
                    "batch submission must not land in the proof namespace"
                );
                if let Some(found) = blobs
                    .batch_blobs
                    .iter_mut()
                    .find(|item| item.commitment() == response.blob_hash.0)
                {
                    assert_eq!(found.sender(), signer);
                    found.advance(found.total_len());
                    assert_eq!(found.verified_data(), blob.as_slice());
                    matched = true;
                    break;
                }
            }
            if matched {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        anyhow::ensure!(matched, "submitted Fibre blob was not found");
        Ok(())
    }
}

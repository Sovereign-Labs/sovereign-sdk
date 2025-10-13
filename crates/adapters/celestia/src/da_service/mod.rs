mod client;
#[cfg(test)]
mod tests;

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use backon::ExponentialBuilder;
use celestia_rpc::prelude::*;
use celestia_types::nmt::Namespace;
use celestia_types::state::Address;
use futures::stream::BoxStream;
use futures::StreamExt;
use jsonrpsee::http_client::HttpClient;
use sov_rollup_interface::da::{DaProof, DaSpec, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, DaService, MaybeRetryable, SubmitBlobReceipt,
};
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::{debug, instrument, trace};

pub use crate::config::CelestiaConfig;
pub use crate::da_service::client::standard_node::StandardNodeClient;
pub use crate::da_service::client::twinkle::TwinkleClient;
pub use crate::da_service::client::CelestiaClient;
use crate::metrics::{GetBlockMeasurement, NamespaceDataMetrics};
use crate::types::{
    BlobWithSender, FilteredCelestiaBlock, NamespaceBoundaryProof, NamespaceRelevantData,
    RollupNamespace,
};
use crate::verifier::address::CelestiaAddress;
use crate::verifier::proofs::{self, BlobProof};
use crate::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams};
use crate::CelestiaHeader;

type BoxError = anyhow::Error;

#[derive(Debug, Clone)]
pub struct CelestiaService {
    // Client is used for a submission request, where we want to have consistent ordering.
    client: CelestiaClient,
    // Client used for queries, where it is not important to have ordering
    read_client: Arc<HttpClient>,
    rollup_batch_namespace: RollupNamespace,
    rollup_proof_namespace: RollupNamespace,
    signer_address: CelestiaAddress,
    safe_lead_time: Duration,
    backoff_policy: ExponentialBuilder,
    request_timeout: Duration,
}

impl CelestiaService {
    fn with_client(
        submit_client: CelestiaClient,
        read_client: HttpClient,
        rollup_batch_namespace: Namespace,
        rollup_proof_namespace: Namespace,
        signer_address: CelestiaAddress,
        safe_lead_time: Duration,
        backoff_policy: ExponentialBuilder,
        request_timeout: Duration,
    ) -> Self {
        Self {
            client: submit_client,
            read_client: Arc::new(read_client),
            rollup_batch_namespace: RollupNamespace::Batch(rollup_batch_namespace),
            rollup_proof_namespace: RollupNamespace::Proof(rollup_proof_namespace),
            signer_address,
            safe_lead_time,
            backoff_policy,
            request_timeout,
        }
    }
}

impl CelestiaService {
    pub async fn new(config: CelestiaConfig, chain_params: RollupParams) -> Self {
        let request_timeout = config.request_timeout();
        let backoff_policy = config.get_backoff_policy();

        let submit_client = config.construct_celestia_client();

        let read_client = config.construct_rpc_client();

        let fetched_address = run_maybe_retryable_async_fn_with_retries(
            backoff_policy,
            || async {
                read_client
                    .state_account_address()
                    .await
                    .map_err(into_transient_with_context)
            },
            "state_account_address",
        )
        .await
        .expect("Failed to query state.AccountAddress to retrieve signer address");

        let fetched_signer = match fetched_address {
            Address::AccAddress(acc) => CelestiaAddress(acc),
            Address::ValAddress(addr) => {
                panic!("Need account address, got validator: {addr}");
            }
            Address::ConsAddress(addr) => {
                panic!("Need account address, got consensus node: {addr}");
            }
        };
        debug!(address = %fetched_signer, "Fetched signer.");

        if let Some(config_signer_address) = config.signer_address {
            if config_signer_address != fetched_signer {
                panic!(
                    "Signer address in in config {config_signer_address} does not match signer address fetched from node {fetched_signer}"
                );
            }
        }

        Self::with_client(
            submit_client,
            read_client,
            chain_params.rollup_batch_namespace,
            chain_params.rollup_proof_namespace,
            fetched_signer,
            Duration::from_millis(config.safe_lead_time_ms),
            backoff_policy,
            request_timeout,
        )
    }
}

/// Allows consuming the [`futures::Stream`] of BlockHeaders.
type HeaderStream = BoxStream<'static, Result<CelestiaHeader, anyhow::Error>>;

impl CelestiaService {
    async fn get_block_at_inner(
        &self,
        height: u64,
    ) -> Result<FilteredCelestiaBlock, MaybeRetryable<anyhow::Error>> {
        let client = &self.read_client;

        // Fetch the header and relevant shares via RPC
        let start_get_block = Instant::now();
        let header = client
            .header_get_by_height(height)
            .await
            .map_err(into_transient_with_context)?;
        let fetch_header_time = start_get_block.elapsed();
        let square_width = header.dah.square_width();
        trace!(%header, height, time_ms = fetch_header_time.as_millis(), "Got the block header");

        let data_futures_all = Instant::now();

        let rollup_batch_rows_future =
            client.share_get_namespace_data(&header, self.rollup_batch_namespace.id());
        let rollup_proof_rows_future =
            client.share_get_namespace_data(&header, self.rollup_proof_namespace.id());

        let (batch_rows, proof_rows) =
            tokio::try_join!(rollup_batch_rows_future, rollup_proof_rows_future,)
                .map_err(into_transient_with_context)?;
        let fetch_rows_time = data_futures_all.elapsed();
        trace!(
            time_ms = data_futures_all.elapsed().as_millis(),
            "All data futures are resolved"
        );

        let build_relevant_data_start = std::time::Instant::now();
        let batch_ns_metrics = NamespaceDataMetrics::new(&batch_rows);
        let rollup_batch_shares =
            NamespaceRelevantData::new(self.rollup_batch_namespace.id(), batch_rows);

        let proof_ns_metrics = NamespaceDataMetrics::new(&proof_rows);
        let rollup_proof_shares =
            NamespaceRelevantData::new(self.rollup_proof_namespace.id(), proof_rows);
        let build_relevant_data = build_relevant_data_start.elapsed();

        let total_time = start_get_block.elapsed();
        trace!(time_ms = total_time.as_millis(), "Get block total");

        sov_metrics::track_metrics(|tracker| {
            let get_block_measurement = GetBlockMeasurement {
                height,
                square_width,
                fetch_header_time,
                fetch_rows_time,
                build_relevant_data,
                batch_ns_metrics,
                proof_ns_metrics,
                total_time,
            };
            tracker.submit(get_block_measurement);
        });

        FilteredCelestiaBlock::new(rollup_batch_shares, rollup_proof_shares, header)
            .map_err(MaybeRetryable::Permanent)
    }

    async fn get_proofs_at_inner(
        &self,
        height: u64,
    ) -> Result<Vec<Vec<u8>>, MaybeRetryable<anyhow::Error>> {
        self.read_client
            .blob_get_all(height, &[self.rollup_proof_namespace.id()])
            .await
            .map_err(into_transient_with_context)
            .map(|blobs| match blobs {
                Some(blobs) => blobs.into_iter().map(|blob| blob.data).collect(),
                None => vec![],
            })
    }

    /// Subscribe to finalized headers as they are finalized.
    /// Expect only to receive headers which were finalized after subscription
    /// Optimized version of `get_last_finalized_block_header`.
    pub async fn subscribe_finalized_header(&self) -> Result<HeaderStream, anyhow::Error> {
        Ok(self
            .read_client
            .header_subscribe()
            .await?
            .map(|res| res.map(CelestiaHeader::from).map_err(|e| e.into()))
            .boxed())
    }
}

fn into_transient_with_context(
    error: jsonrpsee::core::ClientError,
) -> MaybeRetryable<anyhow::Error> {
    let error = anyhow::anyhow!("Celestia RPC node returned an error: {:?}", error);
    MaybeRetryable::Transient(error)
}

#[async_trait]
impl DaService for CelestiaService {
    type Spec = CelestiaSpec;
    type Config = CelestiaConfig;
    type Verifier = CelestiaVerifier;
    type FilteredBlock = FilteredCelestiaBlock;
    type Error = BoxError;

    #[instrument(skip(self))]
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let f = || async {
            tokio::time::timeout(self.request_timeout, self.get_block_at_inner(height))
                .await
                .map_err(|e| MaybeRetryable::Transient(anyhow::anyhow!("Request timeout: {:?}", e)))
                .and_then(|result| result)
        };
        run_maybe_retryable_async_fn_with_retries(self.backoff_policy, f, "get_block_at").await
    }

    #[instrument(skip(self))]
    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.client.get_block_header_at(height).await
    }

    fn safe_lead_time(&self) -> Duration {
        self.safe_lead_time
    }

    #[instrument(skip(self))]
    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        // Tendermint has instant finality, so the head block is the one that finalized
        // and network is always guaranteed to be secure,
        // it can work even if the node is still catching up.
        self.get_head_block_header().await
    }

    #[instrument(skip(self))]
    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.client.get_head_block_header().await
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
        get_extraction_proof(block, blobs)
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        self.client
            .submit_blob_to_namespace(blob, self.rollup_batch_namespace, &self.signer_address)
            .await
    }

    async fn send_proof(
        &self,
        aggregated_proof: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        self.client
            .submit_blob_to_namespace(
                aggregated_proof,
                self.rollup_proof_namespace,
                &self.signer_address,
            )
            .await
    }

    #[instrument(err)]
    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_proofs_at_inner(height),
            "get_proofs_at",
        )
        .await
    }

    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address {
        self.signer_address.clone()
    }
}

pub(crate) fn extract_relevant_blobs(
    block: &FilteredCelestiaBlock,
) -> RelevantBlobs<BlobWithSender> {
    let proof_blobs = block.rollup_proof_data.get_blobs_with_sender();
    let batch_blobs = block.rollup_batch_data.get_blobs_with_sender();
    RelevantBlobs {
        proof_blobs,
        batch_blobs,
    }
}

pub(crate) fn get_extraction_proof(
    block: &FilteredCelestiaBlock,
    blobs: &RelevantBlobs<BlobWithSender>,
) -> RelevantProofs<Vec<BlobProof>, Option<NamespaceBoundaryProof>> {
    let batch = {
        let inclusion_proof = proofs::new_inclusion_proof(
            &block.header,
            &block.rollup_batch_data,
            &blobs.batch_blobs,
        );

        DaProof {
            inclusion_proof,
            completeness_proof: NamespaceBoundaryProof::from_namespace_data(
                &block.rollup_batch_data,
            ),
        }
    };

    let proof = {
        // Note: The second call to new_inclusion_proof merklizes and parse the executable transactions namespace again.
        let inclusion_proof = proofs::new_inclusion_proof(
            &block.header,
            &block.rollup_proof_data,
            &blobs.proof_blobs,
        );

        DaProof {
            inclusion_proof,
            completeness_proof: NamespaceBoundaryProof::from_namespace_data(
                &block.rollup_proof_data,
            ),
        }
    };

    RelevantProofs { proof, batch }
}

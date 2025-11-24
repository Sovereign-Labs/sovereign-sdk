#[cfg(test)]
mod tests;

pub use crate::config::CelestiaConfig;
use crate::metrics::client::{
    GetBlockHeaderMeasurement, GetChainHeadMeasurement, GetNamespaceDataMeasurement,
    SubmitPayForBlob,
};
use crate::metrics::full::{BlobSubmitMeasurement, GetBlockMeasurement, NamespaceDataMetrics};
use crate::metrics::RollupNamespace;
use crate::types::{
    BlobWithSender, FilteredCelestiaBlock, NamespaceBoundaryProof, NamespaceRelevantData, TmHash,
    APP_VERSION,
};
use crate::verifier::address::CelestiaAddress;
use crate::verifier::proofs::{self, BlobProof};
use crate::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams};
use crate::CelestiaHeader;
use anyhow::Context;
use async_trait::async_trait;
use backon::ExponentialBuilder;
use celestia_types::blob::Blob as JsonBlob;
use celestia_types::nmt::Namespace;
use celestia_types::row_namespace_data::NamespaceData;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{DaProof, DaSpec, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, DaService, MaybeRetryable, SubmitBlobReceipt,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::instrument;

type BoxError = anyhow::Error;

#[derive(Debug, Clone)]
pub struct CelestiaService {
    // Client is used for a submission request, where we want to have consistent ordering.
    submit_client: Arc<tokio::sync::Mutex<celestia_client::Client>>,
    // Client used for queries, where it is not important to have ordering
    read_client: Arc<celestia_client::Client>,
    rollup_batch_namespace: Namespace,
    rollup_proof_namespace: Namespace,
    signer_address: Option<CelestiaAddress>,
    safe_lead_time: Duration,
    backoff_policy: ExponentialBuilder,
    request_timeout: Duration,
    tx_priority: celestia_client::tx::TxPriority,
}

impl CelestiaService {
    fn with_client(
        read_client: celestia_client::Client,
        submit_client: celestia_client::Client,
        rollup_batch_namespace: Namespace,
        rollup_proof_namespace: Namespace,
        signer_address: Option<CelestiaAddress>,
        safe_lead_time: Duration,
        backoff_policy: ExponentialBuilder,
        request_timeout: Duration,
        tx_priority: celestia_client::tx::TxPriority,
    ) -> Self {
        Self {
            submit_client: Arc::new(tokio::sync::Mutex::new(submit_client)),
            read_client: Arc::new(read_client),
            rollup_batch_namespace,
            rollup_proof_namespace,
            signer_address,
            safe_lead_time,
            backoff_policy,
            request_timeout,
            tx_priority,
        }
    }

    fn rollup_namespace(&self, namespace: &Namespace) -> RollupNamespace {
        if namespace == &self.rollup_batch_namespace {
            RollupNamespace::Batch
        } else if namespace == &self.rollup_proof_namespace {
            RollupNamespace::Proof
        } else {
            panic!("Passed unknown namespace: {namespace:?}. Bug and misconfiguration")
        }
    }

    fn get_tx_config(&self) -> celestia_client::tx::TxConfig {
        let mut tx_config = celestia_client::tx::TxConfig::default();
        tx_config = tx_config.with_priority(self.tx_priority);
        tx_config
    }

    #[instrument(skip(self, blob, namespace))]
    async fn submit_blob_to_namespace(
        &self,
        blob: &[u8],
        namespace: Namespace,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let start = std::time::Instant::now();
        let bytes = blob.len();
        let ns = self.rollup_namespace(&namespace);
        tracing::debug!(bytes, namespace = ?ns, "Sending raw data to Celestia");

        let Some(signer) = &self.signer_address else {
            // TODO: Follow up: Better error when switched to `thiserror`.
            anyhow::bail!("Signer must be set for submitting blobs");
        };
        let blob = JsonBlob::new(namespace, blob.to_vec(), Some(signer.0), APP_VERSION)
            .expect("Bug in CelestiaAdapter");
        let blob_hash = HexHash::new(*blob.commitment.hash());
        tracing::debug!(
            namespace = ?ns,
            commitment = %blob_hash,
            bytes,
            data_bytes = blob.data.len(),
            "Submitting a blob"
        );

        let start_lock = std::time::Instant::now();
        let submit_client = self.submit_client.lock().await;
        let lock_acquisition = start_lock.elapsed();

        let start_submit = std::time::Instant::now();
        let blobs = &[blob];
        let method_name = format!("submit_{ns}_blob");
        let tx_response = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || async {
                let tx_config = self.get_tx_config();
                let start = std::time::Instant::now();
                let result = tokio::time::timeout(
                    self.request_timeout,
                    submit_client.state().submit_pay_for_blob(blobs, tx_config),
                )
                .await;
                let result = flatten_timeout(result);
                let is_success = result.is_ok();
                let measurement = SubmitPayForBlob::new(ns, start.elapsed(), is_success);
                sov_metrics::track_metrics(|tracker| tracker.submit(measurement));
                result
            },
            &method_name,
        )
        .await?;
        drop(submit_client);

        let submit_time = start_submit.elapsed();
        let total_time = start.elapsed();
        let measurement = BlobSubmitMeasurement {
            namespace: ns,
            bytes,
            lock_acquisition_time: lock_acquisition,
            submit_time,
            total_time,
        };
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(measurement);
        });

        let tx_hash = TmHash(tx_response.hash);
        tracing::info!(
            da_height = tx_response.height.value(),
            tx_hash = %tx_hash,
            blob_hash = %blob_hash,
            bytes,
            namespace = ?ns,
            ?lock_acquisition,
            ?submit_time,
            ?total_time,
            "Blob has been submitted to Celestia"
        );

        Ok(SubmitBlobReceipt {
            blob_hash,
            da_transaction_id: tx_hash,
        })
    }
}

impl CelestiaService {
    pub async fn new(config: CelestiaConfig, chain_params: RollupParams) -> Self {
        tracing::info!(?config, "Initializing Celestia Adapter");
        let request_timeout = Duration::from_secs(config.request_timeout_secs.get());

        let backoff_policy = config.get_backoff_policy();

        let read_client = config
            .build_client()
            .await
            .expect("Failed to build read client");
        let submit_client = config
            .build_client()
            .await
            .expect("Failed to build submit client");

        let fetched_signer = submit_client.address().ok().map(CelestiaAddress);
        if fetched_signer.is_none() {
            tracing::info!(
                "CelestiaService is configured as read-only and won't be able to submit blobs"
            );
        }

        Self::with_client(
            read_client,
            submit_client,
            chain_params.rollup_batch_namespace,
            chain_params.rollup_proof_namespace,
            fetched_signer,
            Duration::from_millis(config.safe_lead_time_ms),
            backoff_policy,
            request_timeout,
            config.tx_priority.into(),
        )
    }
}

/// Allows consuming the [`futures::Stream`] of BlockHeaders.
type HeaderStream = BoxStream<'static, Result<CelestiaHeader, anyhow::Error>>;

impl CelestiaService {
    async fn get_block_header_at_inner(
        &self,
        height: u64,
    ) -> Result<CelestiaHeader, MaybeRetryable<anyhow::Error>> {
        tracing::trace!(height, "Making call to header.GetByHeight");
        let start = std::time::Instant::now();
        let client = &self.read_client;
        let result =
            tokio::time::timeout(self.request_timeout, client.header().get_by_height(height)).await;
        let response_time = start.elapsed();
        let is_success = matches!(result, Ok(Ok(_)));
        tracing::trace!(
            height,
            is_success,
            ?response_time,
            "Call to header.GetByHeight is completed"
        );
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(GetBlockHeaderMeasurement::new(response_time, is_success));
        });
        let extended_header = flatten_timeout(result)?;
        Ok(extended_header.into())
    }

    async fn get_namespace_data_at_inner(
        &self,
        height: u64,
        namespace: Namespace,
    ) -> Result<NamespaceData, MaybeRetryable<anyhow::Error>> {
        let start = std::time::Instant::now();
        let client = &self.read_client;
        let ns = self.rollup_namespace(&namespace);
        tracing::trace!(height, %ns, "Making call to share.GetNamespaceData");
        let result = tokio::time::timeout(
            self.request_timeout,
            client.share().get_namespace_data(height, namespace),
        )
        .await;
        let is_success = matches!(result, Ok(Ok(_)));
        let response_time = start.elapsed();
        tracing::trace!(height, %ns, ?is_success, ?response_time, "Call to share.GetNamespaceData is completed");
        let measurement = GetNamespaceDataMeasurement::new(ns, response_time, is_success);
        sov_metrics::track_metrics(|tracker| tracker.submit(measurement));
        flatten_timeout(result)
    }

    async fn get_block_at_with_retries(
        &self,
        height: u64,
    ) -> anyhow::Result<FilteredCelestiaBlock> {
        tracing::trace!(height, "Getting block, firing requests");
        let start_get_block = Instant::now();

        let header_future = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_block_header_at_inner(height),
            "get_block_header_at",
        );
        let rollup_batch_rows_future = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_namespace_data_at_inner(height, self.rollup_batch_namespace),
            "get_rollup_batch_namespace",
        );
        let rollup_proof_rows_future = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_namespace_data_at_inner(height, self.rollup_proof_namespace),
            "get_rollup_proof_namespace",
        );

        let (header, batch_rows, proof_rows) = tokio::try_join!(
            header_future,
            rollup_batch_rows_future,
            rollup_proof_rows_future,
        )?;
        let futures_time = start_get_block.elapsed();
        tracing::trace!(height, time = ?futures_time, "All requests have been completed, building relevant data..");

        let square_width = header.dah.square_width();

        let build_relevant_data_start = std::time::Instant::now();
        let batch_ns_metrics = NamespaceDataMetrics::new(&batch_rows);
        let rollup_batch_shares =
            NamespaceRelevantData::new(self.rollup_batch_namespace, batch_rows);

        let proof_ns_metrics = NamespaceDataMetrics::new(&proof_rows);
        let rollup_proof_shares =
            NamespaceRelevantData::new(self.rollup_proof_namespace, proof_rows);
        let build_relevant_data = build_relevant_data_start.elapsed();

        let total_time = start_get_block.elapsed();
        tracing::trace!(time_ms = total_time.as_millis(), "Get block total");

        sov_metrics::track_metrics(|tracker| {
            let get_block_measurement = GetBlockMeasurement {
                height,
                square_width,
                futures_time,
                build_relevant_data,
                batch_ns_metrics,
                proof_ns_metrics,
                total_time,
            };
            tracker.submit(get_block_measurement);
        });
        tracing::trace!(height, "get_block_at metrics send, returning");
        FilteredCelestiaBlock::new(rollup_batch_shares, rollup_proof_shares, header)
    }

    async fn get_head_block_header_inner(
        &self,
    ) -> Result<CelestiaHeader, MaybeRetryable<anyhow::Error>> {
        tracing::trace!("Making call to header.NetworkHead");
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            self.request_timeout,
            self.read_client.header().network_head(),
        )
        .await;
        let response_time = start.elapsed();
        let is_success = matches!(result, Ok(Ok(_)));
        tracing::trace!(
            is_success,
            ?response_time,
            "Call to header.NetworkHead is completed"
        );
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(GetChainHeadMeasurement::new(response_time, is_success));
        });
        let header = flatten_timeout(result)?;

        Ok(CelestiaHeader::from(header))
    }

    async fn get_proofs_at_inner(
        &self,
        height: u64,
    ) -> Result<Vec<Vec<u8>>, MaybeRetryable<anyhow::Error>> {
        // TODO: follow up: timeout here
        // TODO: follow up: metrics here
        self.read_client
            .blob()
            .get_all(height, &[self.rollup_proof_namespace])
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
            .header()
            .subscribe()
            .map(|res| res.map(CelestiaHeader::from).map_err(|e| e.into()))
            .boxed())
    }
}

fn into_transient_with_context(error: celestia_client::Error) -> MaybeRetryable<anyhow::Error> {
    // TODO: Follow up: Can be improved on when to retry or not
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
        self.get_block_at_with_retries(height).await
    }

    #[instrument(skip(self))]
    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_block_header_at_inner(height),
            "get_block_header_at",
        )
        .await
    }

    fn safe_lead_time(&self) -> Duration {
        self.safe_lead_time
    }

    #[instrument(skip(self))]
    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        // Tendermint has instant finality, so head block is the one that finalized
        // and network is always guaranteed to be secure,
        // it can work even if the node is still catching up.
        self.get_head_block_header().await
    }

    #[instrument(skip(self))]
    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_head_block_header_inner(),
            "get_head_block_header",
        )
        .await
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        // NOTE: Does not add logic here, it should go directly into the function below,
        // otherwise tests won't cover the change
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
        // NOTE: Does not add logic here, it should go directly into the function below,
        // otherwise tests won't cover the change
        get_extraction_proof(block, blobs)
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();
        let res = self
            .submit_blob_to_namespace(blob, self.rollup_batch_namespace)
            .await
            .context("Batch submit");
        // UNWRAP: Not possible because the receiver is in the scope still
        tx.send(res).unwrap();
        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();
        let res = self
            .submit_blob_to_namespace(aggregated_proof, self.rollup_proof_namespace)
            .await
            .context("Proof submit");
        // UNWRAP: Not possible because the receiver is in the scope still
        tx.send(res).unwrap();
        rx
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

    async fn get_signer(&self) -> Option<<Self::Spec as DaSpec>::Address> {
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

fn flatten_timeout<T>(
    response: Result<Result<T, celestia_client::Error>, tokio::time::error::Elapsed>,
) -> Result<T, MaybeRetryable<anyhow::Error>> {
    match response {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(e)) => Err(MaybeRetryable::Transient(e.into())),
        Err(_) => Err(MaybeRetryable::Transient(anyhow::anyhow!("await timeout"))),
    }
}

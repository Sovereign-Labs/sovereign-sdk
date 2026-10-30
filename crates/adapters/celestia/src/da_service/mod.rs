#[cfg(test)]
mod tests;

pub use crate::config::{CelestiaConfig, VerifyOnFetchMode};
use crate::fibre::{encode_for_proofs, parse_v2_pointer, FIBRE_SHARE_VERSION};
use crate::fibre_archive::{ChainedArchive, FibreArchive, FsArchive, HttpFallbackArchive};
use crate::metrics::client::{
    BlobGetAllMeasurement, GetBlockHeaderMeasurement, GetChainHeadMeasurement,
    GetNamespaceDataMeasurement, HeaderSyncStateMeasurement, StateBalanceForAddressMeasurement,
    StateEstimateGasPriceMeasurement, SubmitPayForBlob,
};
use crate::metrics::full::{
    BlobCompressionMeasurement, BlobSubmitMeasurement, CelestiaAdapterStateMeasurement,
    FibreDownloadMeasurement, FibreEncodeMeasurement, GetBlockMeasurement, NamespaceDataMetrics,
    SubmitPayForFibreMeasurement,
};
use crate::metrics::RollupNamespace;
use crate::types::{
    BlobWithSender, FibreBlobData, FilteredCelestiaBlock, NamespaceBoundaryProof,
    NamespaceRelevantData, TmHash,
};
use crate::verifier::address::CelestiaAddress;
use crate::verifier::proofs::{self, BlobProof};
use crate::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams};
use crate::CelestiaHeader;
use anyhow::Context;
use async_trait::async_trait;
use backon::ExponentialBuilder;
use celestia_client::api::fibre::{BlobID, DownloadOptions};
use celestia_types::blob::Blob as JsonBlob;
use celestia_types::namespace_data::NamespaceData;
use celestia_types::nmt::Namespace;
use celestia_types::ExtendedHeader;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{
    BlobReaderTrait, DaProof, DaSpec, DaVerifier, RelevantBlobs, RelevantProofs,
};
use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, DaService, MaybeRetryable, SubmitBlobReceipt,
};
use sov_shutdown::SecondaryShutdownController;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::instrument;

type BoxError = anyhow::Error;

/// Fibre blobs of a block downloaded and encoded at the same time. Encoding a blob of up to
/// 128 MiB holds several times its size in memory, so blobs are processed one at a time.
const FIBRE_DOWNLOAD_CONCURRENCY: usize = 1;

#[derive(Clone)]
struct FibreArchiveHandle(Arc<dyn FibreArchive>);

impl std::fmt::Debug for FibreArchiveHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FibreArchive")
    }
}

#[derive(Debug, Clone)]
pub struct CelestiaService {
    client: Arc<celestia_client::Client>,
    rollup_batch_namespace: Namespace,
    rollup_proof_namespace: Namespace,
    verify_on_fetch_mode: VerifyOnFetchMode,
    signer_address: Option<CelestiaAddress>,
    safe_lead_time: Duration,
    backoff_policy: ExponentialBuilder,
    request_timeout: Duration,
    tx_priority: celestia_client::tx::TxPriority,
    tx_status_polling_millis: u64,
    compression: crate::config::CompressOnSubmit,
    compression_chunk_size: usize,
    fibre_activation_height: Option<u64>,
    fibre_download_timeout: Duration,
    fibre_archive: Option<FibreArchiveHandle>,
    /// Set only when `fibre.submit` is enabled: batch blobs are then submitted through Fibre.
    fibre_signing_key: Option<celestia_client::tx::SigningKey>,
}

impl CelestiaService {
    // Private all-fields constructor; the field count is inherent to the service config.
    #[allow(clippy::too_many_arguments)]
    fn with_client(
        client: celestia_client::Client,
        rollup_batch_namespace: Namespace,
        rollup_proof_namespace: Namespace,
        verify_on_fetch_mode: VerifyOnFetchMode,
        signer_address: Option<CelestiaAddress>,
        safe_lead_time: Duration,
        backoff_policy: ExponentialBuilder,
        request_timeout: Duration,
        tx_priority: celestia_client::tx::TxPriority,
        tx_status_polling_millis: u64,
        compression: crate::config::CompressOnSubmit,
        compression_chunk_size: usize,
        fibre_activation_height: Option<u64>,
        fibre_download_timeout: Duration,
        fibre_archive: Option<FibreArchiveHandle>,
        fibre_signing_key: Option<celestia_client::tx::SigningKey>,
    ) -> Self {
        Self {
            client: Arc::new(client),
            rollup_batch_namespace,
            rollup_proof_namespace,
            verify_on_fetch_mode,
            signer_address,
            safe_lead_time,
            backoff_policy,
            request_timeout,
            tx_priority,
            tx_status_polling_millis,
            compression,
            compression_chunk_size,
            fibre_activation_height,
            fibre_download_timeout,
            fibre_archive,
            fibre_signing_key,
        }
    }

    fn fibre_active(&self, height: u64) -> bool {
        self.fibre_activation_height
            .is_some_and(|activation| height >= activation)
    }

    /// Whether batch blobs are submitted through Fibre instead of `MsgPayForBlob`.
    pub(crate) fn submits_through_fibre(&self) -> bool {
        self.fibre_signing_key.is_some()
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
        tx_config = tx_config
            .with_priority(self.tx_priority)
            .with_confirmation_interval_ms(self.tx_status_polling_millis);
        tx_config
    }

    #[instrument(skip(self, blob, namespace))]
    async fn submit_blob_to_namespace(
        &self,
        blob: &[u8],
        namespace: Namespace,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let start = std::time::Instant::now();
        let rollup_len = blob.len();
        let ns = self.rollup_namespace(&namespace);

        let Some(signer) = &self.signer_address else {
            // TODO: Follow up: Better error when switched to `thiserror`.
            anyhow::bail!("Signer must be set for submitting blobs");
        };

        // Compress batch blobs when configured; proofs always post verbatim. This is
        // emission only — read/verify semantics never depend on it.
        let da_payload = if matches!(ns, RollupNamespace::Batch) {
            let da_payload = crate::envelope::encode_for_submission(
                blob,
                self.compression.is_enabled(),
                self.compression_chunk_size,
            );
            sov_metrics::track_metrics(|tracker| {
                tracker.submit(BlobCompressionMeasurement::new(
                    self.compression,
                    rollup_len,
                    da_payload.len(),
                ));
            });
            da_payload
        } else {
            blob.to_vec()
        };
        let bytes = da_payload.len();
        tracing::debug!(rollup_len, bytes, namespace = ?ns, "Sending data to Celestia");

        if let (RollupNamespace::Batch, Some(signing_key)) = (ns, &self.fibre_signing_key) {
            return self
                .submit_fibre_blob(signing_key, namespace, da_payload, start)
                .await;
        }

        let blob =
            JsonBlob::new(namespace, da_payload, Some(signer.0)).expect("Bug in CelestiaAdapter");
        let blob_hash = HexHash::new(*blob.commitment.hash());
        tracing::debug!(
            namespace = ?ns,
            commitment = %blob_hash,
            bytes,
            "Submitting a blob"
        );

        let lock_acquisition = std::time::Duration::from_secs(0);
        let submit_client = self.client.clone();

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
            da_height = tx_response.height,
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

    /// Submits a batch payload through Fibre. The blob hash is the Fibre commitment, which is
    /// also what the verifier reports as the hash of the resulting blob.
    async fn submit_fibre_blob(
        &self,
        signing_key: &celestia_client::tx::SigningKey,
        namespace: Namespace,
        da_payload: Vec<u8>,
        start: std::time::Instant,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let bytes = da_payload.len();
        let (da_payload, encoded) = tokio::task::spawn_blocking(move || {
            let encoded = encode_for_proofs(&da_payload);
            (da_payload, encoded)
        })
        .await?;
        let commitment = encoded.context("Failed to encode Fibre blob")?.commitment;
        let blob_hash = HexHash::new(commitment);
        let fibre = self.client.fibre().ok_or_else(|| {
            anyhow::anyhow!(
                "`fibre.submit` is enabled but the Celestia client has no Fibre support"
            )
        })?;
        let head_height = sov_rollup_interface::da::BlockHeaderTrait::height(
            &self.get_head_block_header().await?,
        );
        anyhow::ensure!(
            self.fibre_active(head_height.saturating_add(1)),
            "Fibre activation height {:?} has not been reached (DA head is {head_height}): \
             the verifier would ignore this batch",
            self.fibre_activation_height,
        );
        tracing::debug!(commitment = %blob_hash, bytes, "Submitting a Fibre blob");

        let start_submit = std::time::Instant::now();
        // Not retried: a timeout can fire after `MsgPayForFibre` is broadcast, and re-running
        // `put` would then pay for and post the same batch twice.
        let result = tokio::time::timeout(
            self.request_timeout,
            fibre.put(signing_key, namespace, &da_payload),
        )
        .await;
        let result = match result {
            Ok(result) => result.map_err(anyhow::Error::from),
            Err(_) => Err(anyhow::anyhow!("await timeout")),
        };
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(SubmitPayForFibreMeasurement {
                bytes,
                time: start_submit.elapsed(),
                is_success: result.is_ok(),
            })
        });
        let submitted = result.context("Failed to submit the Fibre blob")?;
        // `MsgPayForFibre` is already broadcast here, so confirmation is deliberately not
        // retried either.
        let tx_info = tokio::time::timeout(self.request_timeout, submitted.confirm())
            .await
            .context("Timed out confirming the Fibre transaction")?
            .context("Failed to confirm the Fibre transaction")?;
        let submit_time = start_submit.elapsed();

        if let Some(archive) = &self.fibre_archive {
            let id = BlobID::new(0, commitment);
            if let Err(error) = archive.0.put(&id, &da_payload).await {
                tracing::warn!(?id, ?error, "Failed to archive submitted Fibre blob");
            }
        }

        let tx_hash = TmHash(tx_info.hash);
        tracing::info!(
            da_height = tx_info.height,
            tx_hash = %tx_hash,
            blob_hash = %blob_hash,
            bytes,
            ?submit_time,
            total_time = ?start.elapsed(),
            "Fibre blob has been submitted to Celestia"
        );

        Ok(SubmitBlobReceipt {
            blob_hash,
            da_transaction_id: tx_hash,
        })
    }
}

impl CelestiaService {
    pub async fn new(
        config: CelestiaConfig,
        chain_params: RollupParams,
        secondary_shutdown_controller: &SecondaryShutdownController,
    ) -> Self {
        tracing::info!(?config, "Initializing Celestia Adapter");
        let request_timeout = Duration::from_secs(config.request_timeout_secs.get());

        let backoff_policy = config.get_backoff_policy();

        let client = config
            .build_client()
            .await
            .expect("Failed to build celestia-client");

        let fetched_signer = client.address().ok().map(CelestiaAddress);
        if fetched_signer.is_none() {
            tracing::info!(
                "CelestiaService is configured as read-only and won't be able to submit blobs"
            );
        }

        let tx_priority = config.tx_priority.clone().into();
        if config.background_stat_polling_interval_secs > 0 {
            if let Some(signer) = fetched_signer {
                let bg_client = config
                    .build_client()
                    .await
                    .expect("Failed to build celestia-client for background task");
                let stat_polling_period =
                    Duration::from_secs(config.background_stat_polling_interval_secs);
                // Background stat collection timeout is 2x the individual API request timeout
                let stat_request_timeout =
                    Duration::from_secs(config.api_request_timeout_secs.get() * 2);
                tokio::spawn(stat_collection_task(
                    bg_client,
                    signer,
                    tx_priority,
                    secondary_shutdown_controller.clone(),
                    stat_polling_period,
                    stat_request_timeout,
                ));
            }
        }

        let (fibre_download_timeout, fibre_archive) = build_fibre_archive(&config);
        let fibre_signing_key =
            build_fibre_signing_key(&config, chain_params.fibre_activation_height);

        Self::with_client(
            client,
            chain_params.rollup_batch_namespace,
            chain_params.rollup_proof_namespace,
            config.verify_on_fetch_mode,
            fetched_signer,
            Duration::from_millis(config.safe_lead_time_ms),
            backoff_policy,
            request_timeout,
            tx_priority,
            config.tx_status_polling_millis,
            config.compression,
            config.compression_chunk_size,
            chain_params.fibre_activation_height,
            fibre_download_timeout,
            fibre_archive,
            fibre_signing_key,
        )
    }
}

/// Returns the key Fibre submissions are signed with, if `fibre.submit` is enabled.
///
/// Panics when submission is enabled without a Fibre activation height: the verifier skips
/// share-version-2 blobs, so every submitted batch would be paid for and then ignored.
fn build_fibre_signing_key(
    config: &CelestiaConfig,
    fibre_activation_height: Option<u64>,
) -> Option<celestia_client::tx::SigningKey> {
    if !config.fibre.as_ref().is_some_and(|fibre| fibre.submit) {
        return None;
    }
    assert!(
        fibre_activation_height.is_some(),
        "`fibre.submit` requires `fibre_activation_height` to be set in the rollup params;          enable it only once the activation height is reached"
    );
    let key_hex = config
        .signer_private_key
        .as_ref()
        .expect("`fibre.submit` requires `signer_private_key`, checked by config validation");
    let key_bytes = hex::decode(key_hex).expect("`signer_private_key` is not valid hex");
    Some(
        celestia_client::tx::SigningKey::from_slice(&key_bytes)
            .expect("`signer_private_key` is not a valid secp256k1 key"),
    )
}

/// Returns the Fibre download timeout and the archive built from `config.fibre`.
fn build_fibre_archive(config: &CelestiaConfig) -> (Duration, Option<FibreArchiveHandle>) {
    let Some(fibre) = &config.fibre else {
        return (Duration::default(), None);
    };
    let mut archives: Vec<Arc<dyn FibreArchive>> = Vec::new();
    if let Some(dir) = &fibre.archive_dir {
        archives.push(Arc::new(FsArchive::new(dir.clone())));
    } else {
        tracing::warn!(
            "Fibre is enabled without `fibre.archive_dir`: this node will be unable to sync Fibre \
             blocks older than the validators' shard retention"
        );
    }
    if !fibre.archive_fallback_urls.is_empty() {
        archives.push(Arc::new(
            HttpFallbackArchive::new(fibre.archive_fallback_urls.clone())
                .expect("Failed to build Fibre archive HTTP client"),
        ));
    }
    let archive =
        (!archives.is_empty()).then(|| FibreArchiveHandle(Arc::new(ChainedArchive::new(archives))));
    (
        Duration::from_secs(fibre.download_timeout_secs.get()),
        archive,
    )
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
        let client = &self.client;
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
        if extended_header.header.height.value() != height {
            return Err(MaybeRetryable::Transient(anyhow::anyhow!(
                "Received wrong height {}, when requested {height}",
                extended_header.header.height.value()
            )));
        }
        Ok(extended_header.into())
    }

    async fn get_namespace_data_at_inner(
        &self,
        height: u64,
        namespace: Namespace,
    ) -> Result<NamespaceData, MaybeRetryable<anyhow::Error>> {
        let start = std::time::Instant::now();
        let client = &self.client;
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

        let fibre_blobs = if self.fibre_active(height) {
            self.get_fibre_blobs(height, &batch_rows).await?
        } else {
            Vec::new()
        };

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
        let mut block =
            FilteredCelestiaBlock::new(rollup_batch_shares, rollup_proof_shares, header)?;
        block.fibre_blobs = fibre_blobs;
        match self.verify_on_fetch_mode {
            VerifyOnFetchMode::Off => {}
            VerifyOnFetchMode::LogError => {
                if let Err(error) = self.verify_block_integrity(&block) {
                    tracing::error!(
                        height,
                        ?error,
                        "Celestia block integrity verification failed; continuing because \
                         verify_on_fetch_mode is `log_error`"
                    );
                }
            }
            VerifyOnFetchMode::ReturnError => {
                self.verify_block_integrity(&block).with_context(|| {
                    format!("Celestia block integrity verification failed at height {height}")
                })?;
            }
        }
        Ok(block)
    }

    /// Fetches the off-chain data of every Fibre pointer in `batch_rows`, ordered by share index.
    async fn get_fibre_blobs(
        &self,
        height: u64,
        batch_rows: &NamespaceData,
    ) -> anyhow::Result<Vec<FibreBlobData>> {
        let mut pointers = Vec::new();
        for share_seq in crate::shares::NamespaceDataIterator::new(batch_rows) {
            let first_share = &share_seq.shares[0];
            if first_share.info_byte().map(|info| info.version()) != Some(FIBRE_SHARE_VERSION) {
                continue;
            }
            // celestia-app cannot produce a malformed pointer, and the verifier would reject the
            // block anyway, so fail instead of skipping it.
            let pointer = parse_v2_pointer(first_share).with_context(|| {
                format!(
                    "Malformed Fibre pointer at height {height}, share {}",
                    share_seq.range_in_ns.start
                )
            })?;
            pointers.push((share_seq.range_in_ns.start, pointer.commitment));
        }
        // `buffered` yields results in input order, so the blobs stay sorted by `share_idx_in_ns`.
        futures::stream::iter(pointers)
            .map(|(share_idx_in_ns, commitment)| {
                self.get_fibre_blob(height, share_idx_in_ns, commitment)
            })
            .buffered(FIBRE_DOWNLOAD_CONCURRENCY)
            .try_collect()
            .await
    }

    /// Returns the data behind a Fibre pointer, checked against the pointer's commitment.
    /// The archive is untrusted: data that does not match is discarded and downloaded instead.
    async fn get_fibre_blob(
        &self,
        height: u64,
        share_idx_in_ns: usize,
        commitment: [u8; 32],
    ) -> anyhow::Result<FibreBlobData> {
        let id = BlobID::new(0, commitment);
        if let Some(archive) = &self.fibre_archive {
            let start = std::time::Instant::now();
            match archive.0.get(&id).await {
                Ok(Some(data)) => {
                    sov_metrics::track_metrics(|tracker| {
                        tracker.submit(FibreDownloadMeasurement {
                            bytes: data.len(),
                            response_time: start.elapsed(),
                            is_success: true,
                            from_archive: true,
                        });
                    });
                    match self
                        .encode_fibre_blob(share_idx_in_ns, commitment, data)
                        .await
                    {
                        Ok(blob) => return Ok(blob),
                        Err(error) => tracing::warn!(
                            height,
                            ?id,
                            ?error,
                            "Archived Fibre blob was rejected, downloading it instead"
                        ),
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(height, ?id, ?error, "Fibre archive lookup failed"),
            }
        }

        let data = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.download_fibre_blob_inner(height, &id),
            "fibre_download",
        )
        .await?;
        let blob = self
            .encode_fibre_blob(share_idx_in_ns, commitment, data)
            .await?;
        if let Some(archive) = &self.fibre_archive {
            if let Err(error) = archive.0.put(&id, &blob.data).await {
                tracing::warn!(height, ?id, ?error, "Failed to archive Fibre blob");
            }
        }
        Ok(blob)
    }

    async fn download_fibre_blob_inner(
        &self,
        height: u64,
        id: &BlobID,
    ) -> Result<sov_rollup_interface::Bytes, MaybeRetryable<anyhow::Error>> {
        let fibre = self.client.fibre().ok_or_else(|| {
            MaybeRetryable::Permanent(anyhow::anyhow!(
                "Block {height} has Fibre blobs but the Celestia client has no Fibre support: set `fibre` in the config"
            ))
        })?;
        tracing::trace!(height, ?id, "Downloading Fibre blob");
        let start = std::time::Instant::now();
        let options = DownloadOptions {
            height: Some(height),
            ..Default::default()
        };
        let result = tokio::time::timeout(self.fibre_download_timeout, fibre.download(id, options))
            .await
            .map_err(|_| anyhow::anyhow!("await timeout"))
            .and_then(|downloaded| downloaded.map_err(Into::into));
        let response_time = start.elapsed();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(FibreDownloadMeasurement {
                bytes: result.as_ref().map_or(0, |blob| blob.data().len()),
                response_time,
                is_success: result.is_ok(),
                from_archive: false,
            });
        });
        result
            .map(|blob| sov_rollup_interface::Bytes::copy_from_slice(blob.data()))
            .map_err(MaybeRetryable::Transient)
    }

    /// Re-encodes `data` to recover its proof material and checks it against `commitment`.
    async fn encode_fibre_blob(
        &self,
        share_idx_in_ns: usize,
        commitment: [u8; 32],
        data: sov_rollup_interface::Bytes,
    ) -> anyhow::Result<FibreBlobData> {
        let bytes = data.len();
        let start = std::time::Instant::now();
        let to_encode = data.clone();
        let encoded = tokio::task::spawn_blocking(move || encode_for_proofs(&to_encode)).await??;
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(FibreEncodeMeasurement {
                bytes,
                time: start.elapsed(),
            });
        });
        anyhow::ensure!(
            encoded.commitment == commitment,
            "Fibre data does not match on-chain commitment"
        );
        Ok(FibreBlobData::with_encoded(share_idx_in_ns, data, encoded))
    }

    async fn get_head_block_header_inner(
        &self,
    ) -> Result<CelestiaHeader, MaybeRetryable<anyhow::Error>> {
        tracing::trace!("Making call to header.NetworkHead");
        let start = std::time::Instant::now();
        let result =
            tokio::time::timeout(self.request_timeout, self.client.header().network_head()).await;
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
        tracing::trace!(height, "Making call to blob.GetAll for proofs");
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            self.request_timeout,
            self.client
                .blob()
                .get_all(height, &[self.rollup_proof_namespace]),
        )
        .await;
        let response_time = start.elapsed();
        let is_success = matches!(result, Ok(Ok(_)));
        tracing::trace!(
            is_success,
            ?response_time,
            height,
            "Call to blob.GetAll is completed"
        );
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(BlobGetAllMeasurement::new(response_time, is_success));
        });

        let blobs = flatten_timeout(result)?
            .map(|blobs| {
                blobs
                    .into_iter()
                    // Proofs post verbatim, but decode defensively so any node reads
                    // back the rollup bytes regardless of who emitted the blob.
                    .map(|blob| crate::envelope::decode_for_read(&blob.data))
                    .collect()
            })
            .unwrap_or_default();
        Ok(blobs)
    }

    /// Subscribe to finalized headers as they are finalized.
    /// Expect only to receive headers which were finalized after subscription
    /// Optimized version of `get_last_finalized_block_header`.
    pub async fn subscribe_finalized_header(&self) -> Result<HeaderStream, anyhow::Error> {
        Ok(self
            .client
            .header()
            .subscribe()
            .map(|res| res.map(CelestiaHeader::from).map_err(|e| e.into()))
            .boxed())
    }

    fn verify_block_integrity(&self, block: &FilteredCelestiaBlock) -> anyhow::Result<()> {
        verify_block_integrity_for_params(
            block,
            RollupParams {
                rollup_batch_namespace: self.rollup_batch_namespace,
                rollup_proof_namespace: self.rollup_proof_namespace,
                fibre_activation_height: self.fibre_activation_height,
            },
        )
    }
}

fn verify_block_integrity_for_params(
    block: &FilteredCelestiaBlock,
    rollup_params: RollupParams,
) -> anyhow::Result<()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        verify_block_integrity_inner(block, rollup_params)
    }))
    .map_err(|panic| {
        anyhow::anyhow!(
            "Celestia block integrity verification panicked: {}",
            panic_payload_to_string(panic.as_ref())
        )
    })?
}

fn verify_block_integrity_inner(
    block: &FilteredCelestiaBlock,
    rollup_params: RollupParams,
) -> anyhow::Result<()> {
    let verifier = CelestiaVerifier::new(rollup_params);

    let mut relevant_blobs = extract_relevant_blobs(block);
    // Advance full blob data first, then derive proofs for the consumed ranges.
    for blob in relevant_blobs
        .batch_blobs
        .iter_mut()
        .chain(relevant_blobs.proof_blobs.iter_mut())
    {
        blob.advance(blob.total_len());
    }
    let relevant_proofs = get_extraction_proof(block, &relevant_blobs);

    verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)?;

    Ok(())
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

    /// Generates inclusion and completeness proofs for the provided blobs against the block.
    ///
    /// # Panics
    ///
    /// Panics if proof generation fails due to inconsistent inputs (e.g. the block data
    /// doesn't match the provided blobs). This indicates a bug in extraction plumbing.
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
        self.signer_address
    }

    async fn get_approximate_block_time(&self) -> Duration {
        std::time::Duration::from_secs(3)
    }
}

pub fn extract_relevant_blobs(block: &FilteredCelestiaBlock) -> RelevantBlobs<BlobWithSender> {
    // Fibre data is only fetched once Fibre is active, so its presence tells the activation state.
    let fibre_active = !block.fibre_blobs.is_empty();
    let proof_blobs = block.rollup_proof_data.get_blobs_with_sender(false, &[]);
    let batch_blobs = block
        .rollup_batch_data
        .get_blobs_with_sender(fibre_active, &block.fibre_blobs);
    RelevantBlobs {
        proof_blobs,
        batch_blobs,
    }
}

pub fn get_extraction_proof(
    block: &FilteredCelestiaBlock,
    blobs: &RelevantBlobs<BlobWithSender>,
) -> RelevantProofs<Vec<BlobProof>, Option<NamespaceBoundaryProof>> {
    let batch = {
        let inclusion_proof = proofs::new_inclusion_proof(
            &block.header,
            &block.rollup_batch_data,
            &blobs.batch_blobs,
            &block.fibre_blobs,
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
            &[],
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

#[doc(hidden)]
pub fn filtered_block_from_json_path(
    batch_namespace: Namespace,
    proof_namespace: Namespace,
    path: &Path,
) -> anyhow::Result<FilteredCelestiaBlock> {
    const HEADER_JSON: &str = "header.json";
    const ROLLUP_BATCH_ROWS_JSON: &str = "rollup_batch_rows.json";
    const ROLLUP_PROOF_ROWS_JSON: &str = "rollup_proof_rows.json";

    let header: ExtendedHeader = load_json_from_fixture(path, HEADER_JSON)?;
    let rollup_batch_rows: NamespaceData = load_json_from_fixture(path, ROLLUP_BATCH_ROWS_JSON)?;
    let rollup_proof_rows: NamespaceData = load_json_from_fixture(path, ROLLUP_PROOF_ROWS_JSON)?;

    let rollup_batch_shares = NamespaceRelevantData::new(batch_namespace, rollup_batch_rows);
    let rollup_proof_shares = NamespaceRelevantData::new(proof_namespace, rollup_proof_rows);

    FilteredCelestiaBlock::new(rollup_batch_shares, rollup_proof_shares, header.into())
}

fn load_json_from_fixture<T: serde::de::DeserializeOwned>(
    path: &Path,
    name: &str,
) -> anyhow::Result<T> {
    let path = path.join(name);
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    serde_json::from_str(&contents).with_context(|| format!("Failed to parse {}", path.display()))
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

async fn stat_collection_task(
    client: celestia_client::Client,
    signer: CelestiaAddress,
    priority: celestia_client::tx::TxPriority,
    secondary_shutdown_controller: SecondaryShutdownController,
    period: Duration,
    request_timeout: Duration,
) {
    let chain_id = client.chain_id();
    tracing::info!(%chain_id, ?period, "Starting celestia stat collection task");

    let mut interval = tokio::time::interval(period);

    loop {
        tokio::select! {
            _ = secondary_shutdown_controller.wait_for_shutdown() => {
                tracing::info!("Shutting down celestia stat collection task");
                return;
            }
            _ = interval.tick() => {
                match gather_stat(&client, &signer, priority, request_timeout).await {
                    Ok(measurement) => {
                        sov_metrics::track_metrics(|tracker| {
                            tracker.submit(measurement);
                        });
                    }
                    Err(error) => {
                        tracing::info!(
                            ?error,
                            "Error gathering background statistics for celestia adapter"
                        );
                    }
                }
            }
        }
    }
}

async fn gather_stat(
    client: &celestia_client::Client,
    signer: &CelestiaAddress,
    priority: celestia_client::tx::TxPriority,
    request_timeout: Duration,
) -> anyhow::Result<CelestiaAdapterStateMeasurement> {
    // Balance
    let balance_start = std::time::Instant::now();
    let balance_response = tokio::time::timeout(
        request_timeout,
        client.state().balance_for_address(&signer.0),
    )
    .await;
    let response_time = balance_start.elapsed();
    let is_success = matches!(balance_response, Ok(Ok(_)));
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(StateBalanceForAddressMeasurement::new(
            response_time,
            is_success,
        ));
    });
    let balance = flatten_timeout::<u64>(balance_response).context("state.BalanceForAddress")?;

    // Sync state
    let sync_state_start = std::time::Instant::now();
    let sync_state_response =
        tokio::time::timeout(request_timeout, client.header().sync_state()).await;
    let response_time = sync_state_start.elapsed();
    let is_success = matches!(sync_state_response, Ok(Ok(_)));
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(HeaderSyncStateMeasurement::new(response_time, is_success));
    });
    let sync_state = flatten_timeout(sync_state_response).context("header.SyncState")?;
    let sync_distance = sync_state.to_height.saturating_sub(sync_state.from_height);

    // Gas price
    let gas_price_start = std::time::Instant::now();
    let gas_price_response =
        tokio::time::timeout(request_timeout, client.state().estimate_gas_price(priority)).await;
    let response_time = gas_price_start.elapsed();
    let is_success = matches!(gas_price_response, Ok(Ok(_)));
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(StateEstimateGasPriceMeasurement::new(
            response_time,
            is_success,
        ));
    });
    let gas_price = flatten_timeout(gas_price_response).context("state.EstimateGasPrice")?;

    Ok(CelestiaAdapterStateMeasurement {
        signer: *signer,
        balance,
        gas_price,
        sync_distance,
    })
}

#[cfg(test)]
mod fibre_submit_tests {
    use super::build_fibre_signing_key;
    use crate::config::{CelestiaConfig, FibreConfig};

    const KEY_HEX: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

    fn config(submit: bool, key: Option<&str>) -> CelestiaConfig {
        let mut config = CelestiaConfig::minimal("ws://localhost:26658".to_string());
        config.signer_private_key = key.map(str::to_string);
        config.fibre = Some(FibreConfig {
            submit,
            ..Default::default()
        });
        config
    }

    #[test]
    fn no_signing_key_when_fibre_submit_is_disabled() {
        let key = build_fibre_signing_key(&config(false, Some(KEY_HEX)), Some(1));
        assert_eq!(key, None);
    }

    #[test]
    fn signing_key_is_built_from_configured_hex() {
        let key = build_fibre_signing_key(&config(true, Some(KEY_HEX)), Some(1))
            .expect("fibre.submit is enabled");
        assert_eq!(key.to_bytes().as_slice(), hex::decode(KEY_HEX).unwrap());
    }

    #[test]
    #[should_panic(expected = "requires `fibre_activation_height`")]
    fn fibre_submit_without_activation_height_is_rejected() {
        build_fibre_signing_key(&config(true, Some(KEY_HEX)), None);
    }
}

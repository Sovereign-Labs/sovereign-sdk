use crate::da_service::into_transient_with_context;
use crate::metrics::{
    BlobSubmitMeasurement, GetBlockHeaderMeasurement, GetBlockMeasurement, GetChainHeadMeasurement,
    NamespaceDataMetrics,
};
use crate::types::{
    FilteredCelestiaBlock, NamespaceRelevantData, RollupNamespace, TmHash, APP_VERSION,
};
use crate::verifier::address::CelestiaAddress;
use crate::CelestiaHeader;
use backon::ExponentialBuilder;
use celestia_rpc::{BlobClient, HeaderClient, ShareClient, StateClient, TxPriority};
use celestia_types::blob::Blob as JsonBlob;
use celestia_types::state::Address;
use jsonrpsee::http_client::HttpClient;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, MaybeRetryable, SubmitBlobReceipt,
};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot::Receiver;
use tokio::sync::{oneshot, Mutex};
use tokio::time::Instant;
use tracing::instrument;

/// Client that communicates with standard celestia data availability nodes,
/// such as light, bridge or full node.
#[derive(Debug, Clone)]
pub struct StandardNodeClient {
    submit_client: Arc<Mutex<HttpClient>>,
    read_client: Arc<HttpClient>,
    backoff_policy: ExponentialBuilder,
    // Separate request timeout, because jsonrpsee is sloppy about it.
    request_timeout: Duration,
    tx_priority: Option<TxPriority>,
    pub(crate) signer_address: CelestiaAddress,
}

impl StandardNodeClient {
    pub async fn new(
        client: HttpClient,
        tx_priority: Option<TxPriority>,
        request_timeout: Duration,
        backoff_policy: ExponentialBuilder,
        signer_address: Option<CelestiaAddress>,
    ) -> Self {
        let fetched_address = run_maybe_retryable_async_fn_with_retries(
            backoff_policy,
            || async {
                client
                    .state_account_address()
                    .await
                    .map_err(into_transient_with_context)
            },
            "state_account_address",
        )
        .await
        .expect("Failed to query `state.AccountAddress` to retrieve signer address");
        let fetched_signer = match fetched_address {
            Address::AccAddress(acc) => CelestiaAddress(acc),
            Address::ValAddress(addr) => {
                panic!("Need account address, got validator: {addr}");
            }
            Address::ConsAddress(addr) => {
                panic!("Need account address, got consensus node: {addr}");
            }
        };
        tracing::debug!(address = %fetched_signer, "Fetched signer.");
        if let Some(config_signer_address) = signer_address {
            if config_signer_address != fetched_signer {
                panic!(
                    "Signer address in in config {config_signer_address} does not match signer address fetched from node {fetched_signer}"
                );
            }
        }

        Self {
            submit_client: Arc::new(Mutex::new(client.clone())),
            read_client: Arc::new(client),
            backoff_policy,
            request_timeout,
            tx_priority,
            signer_address: fetched_signer,
        }
    }

    #[instrument(skip(self, blob, signer, namespace), fields(namespace = %namespace.ns_type()))]
    async fn submit_blob_to_namespace_inner(
        &self,
        blob: &[u8],
        namespace: RollupNamespace,
        signer: &CelestiaAddress,
    ) -> Result<SubmitBlobReceipt<TmHash>, MaybeRetryable<anyhow::Error>> {
        let start = std::time::Instant::now();
        let bytes = blob.len();

        let blob = JsonBlob::new(
            namespace.id(),
            blob.to_vec(),
            Some(signer.0.clone()),
            APP_VERSION,
        )
        .map_err(|e| MaybeRetryable::Permanent(anyhow::anyhow!("Failed to build blob: {e:?}")))?;

        let blob_hash = HexHash::new(*blob.commitment.hash());
        tracing::debug!(
            commitment = %blob_hash,
            bytes,
            "Submitting a blob"
        );

        let mut tx_config = celestia_rpc::TxConfig::default();
        if let Some(priority) = self.tx_priority.as_ref() {
            tx_config = tx_config.with_priority(*priority);
        }

        let start_lock = std::time::Instant::now();
        let submit_client = self.submit_client.lock().await;
        let lock_acquisition = start_lock.elapsed();

        let start_submit = std::time::Instant::now();
        let tx_result = match tokio::time::timeout(
            self.request_timeout,
            submit_client.state_submit_pay_for_blob(&[blob.into()], tx_config),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => Err(anyhow::anyhow!("Error from state.SubmitPayForBlob: {e:?}",)),
            Err(_) => Err(anyhow::anyhow!(
                "Timeout waiting for state.SubmitPayForBlob"
            )),
        };
        drop(submit_client);

        let submit_time = start_submit.elapsed();
        let total_time = start.elapsed();
        let measurement = BlobSubmitMeasurement::new_for_standard(
            namespace.ns_type(),
            &tx_result,
            bytes,
            lock_acquisition,
            submit_time,
            total_time,
        );
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(measurement);
        });

        let tx_response = tx_result.map_err(MaybeRetryable::Transient)?;
        let tx_hash = TmHash(
            tendermint::Hash::from_str(&tx_response.txhash).map_err(|e| {
                MaybeRetryable::Permanent(anyhow::anyhow!("Failed to convert tx_hash: {e:?}"))
            })?,
        );
        tracing::info!(
            da_height = tx_response.height,
            tx_hash = %tx_hash,
            code = %tx_response.code,
            blob_hash = %blob_hash,
            gas_used = %tx_response.gas_used,
            bytes,
            namespace = ?namespace.ns_type(),
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

    pub async fn submit_blob_to_namespace(
        &self,
        blob: &[u8],
        namespace: RollupNamespace,
    ) -> Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        let (tx, rx) = oneshot::channel();
        let res = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.submit_blob_to_namespace_inner(blob, namespace, &self.signer_address),
            "send_transaction",
        )
        .await;
        if let Err(result) = tx.send(res) {
            tracing::warn!(?result, "Failed to send result of submitting blob to the sender. Probably it is dropped. It is normal during shutdown");
        }
        rx
    }

    async fn get_block_header_at_inner(
        &self,
        height: u64,
    ) -> Result<CelestiaHeader, MaybeRetryable<anyhow::Error>> {
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            self.request_timeout,
            self.read_client.header_get_by_height(height),
        )
        .await
        .map_err(|_| MaybeRetryable::Transient(anyhow::anyhow!("Request timeout")))?
        .map_err(into_transient_with_context);
        let measurement = GetBlockHeaderMeasurement {
            height,
            fetch_header_time: start.elapsed(),
            is_success: result.is_ok(),
        };
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(measurement);
        });
        let extended_header = result?;
        Ok(extended_header.into())
    }

    async fn get_head_block_header_inner(
        &self,
    ) -> Result<CelestiaHeader, MaybeRetryable<anyhow::Error>> {
        let start = std::time::Instant::now();
        let result =
            tokio::time::timeout(self.request_timeout, self.read_client.header_network_head())
                .await
                .map_err(|_| MaybeRetryable::Transient(anyhow::anyhow!("Request timeout")))?
                .map_err(into_transient_with_context);
        let is_success = result.is_ok();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(GetChainHeadMeasurement {
                fetch_header_time: start.elapsed(),
                is_success,
            });
        });
        Ok(CelestiaHeader::from(result?))
    }

    #[instrument(skip(self))]
    pub async fn get_block_header_at(&self, height: u64) -> anyhow::Result<CelestiaHeader> {
        run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_block_header_at_inner(height),
            "get_block_header_at",
        )
        .await
    }

    #[instrument(skip(self))]
    pub async fn get_head_block_header(&self) -> anyhow::Result<CelestiaHeader> {
        run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_head_block_header_inner(),
            "get_head_block_header",
        )
        .await
    }

    async fn get_block_at_inner(
        &self,
        height: u64,
        batch_namespace: &RollupNamespace,
        proof_namespace: &RollupNamespace,
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
        tracing::trace!(%header, height, time_ms = fetch_header_time.as_millis(), "Got the block header");

        let data_futures_all = Instant::now();

        let rollup_batch_rows_future =
            client.share_get_namespace_data(&header, batch_namespace.id());
        let rollup_proof_rows_future =
            client.share_get_namespace_data(&header, proof_namespace.id());

        let (batch_rows, proof_rows) =
            tokio::try_join!(rollup_batch_rows_future, rollup_proof_rows_future,)
                .map_err(into_transient_with_context)?;
        let fetch_rows_time = data_futures_all.elapsed();
        tracing::trace!(
            time_ms = data_futures_all.elapsed().as_millis(),
            "All data futures are resolved"
        );

        let build_relevant_data_start = std::time::Instant::now();
        let batch_ns_metrics = NamespaceDataMetrics::new(&batch_rows);
        let rollup_batch_shares = NamespaceRelevantData::new(batch_namespace.id(), batch_rows);

        let proof_ns_metrics = NamespaceDataMetrics::new(&proof_rows);
        let rollup_proof_shares = NamespaceRelevantData::new(proof_namespace.id(), proof_rows);
        let build_relevant_data = build_relevant_data_start.elapsed();

        let total_time = start_get_block.elapsed();
        tracing::trace!(time_ms = total_time.as_millis(), "Get block total");

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

        Ok(FilteredCelestiaBlock::new(
            rollup_batch_shares,
            rollup_proof_shares,
            header,
        ))
    }

    #[instrument(skip(self))]
    pub async fn get_block_at(
        &self,
        height: u64,
        batch_namespace: &RollupNamespace,
        proof_namespace: &RollupNamespace,
    ) -> anyhow::Result<FilteredCelestiaBlock> {
        let f = || async {
            tokio::time::timeout(
                self.request_timeout,
                self.get_block_at_inner(height, batch_namespace, proof_namespace),
            )
            .await
            .map_err(|e| MaybeRetryable::Transient(anyhow::anyhow!("Request timeout: {:?}", e)))
            .and_then(|result| result)
        };
        run_maybe_retryable_async_fn_with_retries(self.backoff_policy, f, "get_block_at").await
    }

    async fn get_blobs_at_inner(
        &self,
        height: u64,
        namespace: &RollupNamespace,
    ) -> Result<Vec<Vec<u8>>, MaybeRetryable<anyhow::Error>> {
        self.read_client
            .blob_get_all(height, &[namespace.id()])
            .await
            .map_err(into_transient_with_context)
            .map(|blobs| match blobs {
                Some(blobs) => blobs.into_iter().map(|blob| blob.data).collect(),
                None => vec![],
            })
    }

    #[instrument(err)]
    pub async fn get_blobs_at(
        &self,
        height: u64,
        namespace: &RollupNamespace,
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.get_blobs_at_inner(height, namespace),
            "get_blobs_at",
        )
        .await
    }
}

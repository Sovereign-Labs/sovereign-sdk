#![allow(dead_code)]
use crate::da_service::into_transient_with_context;
use crate::metrics::{BlobSubmitMeasurement, RollupNamespace};
use crate::types::{TmHash, APP_VERSION};
use crate::verifier::address::CelestiaAddress;
use crate::CelestiaConfig;
use backon::ExponentialBuilder;
use celestia_rpc::StateClient;
use celestia_types::blob::Blob as JsonBlob;
use celestia_types::nmt::Namespace;
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
use tracing::{debug, info, instrument};

#[derive(Debug, Clone)]
pub struct VanillaClient {
    client: Arc<Mutex<HttpClient>>,
    backoff_policy: ExponentialBuilder,
    // Separate request timeout, because jsonrpsee is sloppy about it.
    request_timeout: Duration,
}

impl VanillaClient {
    pub fn new(config: &CelestiaConfig) -> Self {
        let client = config.construct_rpc_client();

        Self {
            client: Arc::new(Mutex::new(client)),
            backoff_policy: config.get_backoff_policy(),
            request_timeout: config.request_timeout(),
        }
    }

    #[instrument(skip_all)]
    async fn submit_blob_to_namespace_inner(
        &self,
        blob: &[u8],
        namespace_id: Namespace,
        namespace: RollupNamespace,
        signer: &CelestiaAddress,
    ) -> Result<SubmitBlobReceipt<TmHash>, MaybeRetryable<anyhow::Error>> {
        let start = std::time::Instant::now();
        let bytes = blob.len();

        let blob = JsonBlob::new(
            namespace_id,
            blob.to_vec(),
            Some(signer.0.clone()),
            APP_VERSION,
        )
        .expect("Bug in CelestiaAdapter");

        let blob_hash = HexHash::new(*blob.commitment.hash());
        debug!(
            ?namespace,
            commitment = %blob_hash,
            bytes,
            "Submitting a blob"
        );

        let tx_config = celestia_rpc::TxConfig::default();

        let start_lock = std::time::Instant::now();
        let submit_client = self.client.lock().await;
        let lock_acquisition = start_lock.elapsed();

        let start_submit = std::time::Instant::now();
        let tx_result = submit_client
            .state_submit_pay_for_blob(&[blob.into()], tx_config)
            .await;
        drop(submit_client);

        let submit_time = start_submit.elapsed();
        let total_time = start.elapsed();
        let measurement = BlobSubmitMeasurement::new_for_vanilla(
            namespace,
            &tx_result,
            bytes,
            lock_acquisition,
            submit_time,
            total_time,
        );
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(measurement);
        });

        let tx_response = tx_result.map_err(into_transient_with_context)?;
        let tx_hash = TmHash(
            tendermint::Hash::from_str(&tx_response.txhash)
                .expect("Failed to decode hash from `TxResponse`"),
        );
        info!(
            da_height = tx_response.height,
            tx_hash = %tx_hash,
            code = %tx_response.code,
            blob_hash = %blob_hash,
            gas_used = %tx_response.gas_used,
            bytes,
            ?namespace,
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
        namespace_id: Namespace,
        namespace: RollupNamespace,
        signer: &CelestiaAddress,
    ) -> Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        let (tx, rx) = oneshot::channel();
        let res = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.submit_blob_to_namespace_inner(blob, namespace_id, namespace, signer),
            "send_transaction",
        )
        .await;
        tx.send(res).unwrap();
        rx
    }
}

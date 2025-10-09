use crate::metrics::BlobSubmitMeasurement;
use crate::types::{RollupNamespace, TmHash, APP_VERSION};
use crate::verifier::address::CelestiaAddress;
use backon::ExponentialBuilder;
use celestia_rpc::{StateClient, TxPriority};
use celestia_types::blob::Blob as JsonBlob;
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

/// Client that communicates with standard celestia data availability nodes,
/// such as light, bridge or full node.
#[derive(Debug, Clone)]
pub struct StandardNodeClient {
    client: Arc<Mutex<HttpClient>>,
    backoff_policy: ExponentialBuilder,
    // Separate request timeout, because jsonrpsee is sloppy about it.
    request_timeout: Duration,
    tx_priority: Option<TxPriority>,
}

impl StandardNodeClient {
    pub fn new(
        client: HttpClient,
        tx_priority: Option<TxPriority>,
        request_timeout: Duration,
        backoff_policy: ExponentialBuilder,
    ) -> Self {
        Self {
            client: Arc::new(Mutex::new(client)),
            backoff_policy,
            request_timeout,
            tx_priority,
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
        .expect("Bug in CelestiaAdapter");

        let blob_hash = HexHash::new(*blob.commitment.hash());
        debug!(
            commitment = %blob_hash,
            bytes,
            "Submitting a blob"
        );

        let mut tx_config = celestia_rpc::TxConfig::default();
        if let Some(priority) = self.tx_priority.as_ref() {
            tx_config = tx_config.with_priority(*priority);
        }

        let start_lock = std::time::Instant::now();
        let submit_client = self.client.lock().await;
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
        signer: &CelestiaAddress,
    ) -> Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        let (tx, rx) = oneshot::channel();
        let res = run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || self.submit_blob_to_namespace_inner(blob, namespace, signer),
            "send_transaction",
        )
        .await;
        tx.send(res).unwrap();
        rx
    }
}

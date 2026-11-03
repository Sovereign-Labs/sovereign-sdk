use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use celestia_types::namespace_data::NamespaceData;
use celestia_types::nmt::Namespace;
use futures::stream::BoxStream;
use futures::StreamExt;

use crate::config::VerifyOnFetchMode;
use crate::header::CelestiaHeader;
use crate::metrics::{
    GetBlockHeaderMeasurement, GetBlockMeasurement, GetChainHeadMeasurement,
    GetNamespaceDataMeasurement, NamespaceDataMetrics, RollupNamespace,
};

pub use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, MaybeRetryable,
};

pub type HeaderStream = BoxStream<'static, Result<CelestiaHeader, anyhow::Error>>;

#[derive(Clone, Debug)]
pub struct CelestiaNode {
    pub client: Arc<celestia_client::Client>,
    pub request_timeout: Duration,
    pub backoff: backon::ExponentialBuilder,
}

impl CelestiaNode {
    pub fn new(
        client: celestia_client::Client,
        request_timeout: Duration,
        backoff: backon::ExponentialBuilder,
    ) -> Self {
        Self {
            client: Arc::new(client),
            request_timeout,
            backoff,
        }
    }

    pub async fn get_header_at(
        &self,
        height: u64,
    ) -> Result<CelestiaHeader, MaybeRetryable<anyhow::Error>> {
        tracing::trace!(height, "Making call to header.GetByHeight");
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            self.request_timeout,
            self.client.header().get_by_height(height),
        )
        .await;
        let response_time = start.elapsed();
        let is_success = matches!(result, Ok(Ok(_)));
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

    pub async fn get_head_header(&self) -> Result<CelestiaHeader, MaybeRetryable<anyhow::Error>> {
        tracing::trace!("Making call to header.NetworkHead");
        let start = std::time::Instant::now();
        let result =
            tokio::time::timeout(self.request_timeout, self.client.header().network_head()).await;
        let response_time = start.elapsed();
        let is_success = matches!(result, Ok(Ok(_)));
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(GetChainHeadMeasurement::new(response_time, is_success));
        });
        Ok(CelestiaHeader::from(flatten_timeout(result)?))
    }

    pub async fn get_namespace_data_at(
        &self,
        height: u64,
        namespace: Namespace,
        label: RollupNamespace,
    ) -> Result<NamespaceData, MaybeRetryable<anyhow::Error>> {
        let start = std::time::Instant::now();
        tracing::trace!(height, %label, "Making call to share.GetNamespaceData");
        let result = tokio::time::timeout(
            self.request_timeout,
            self.client.share().get_namespace_data(height, namespace),
        )
        .await;
        let is_success = matches!(result, Ok(Ok(_)));
        let response_time = start.elapsed();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(GetNamespaceDataMeasurement::new(
                label,
                response_time,
                is_success,
            ));
        });
        flatten_timeout(result)
    }

    pub async fn subscribe_headers(&self) -> Result<HeaderStream, anyhow::Error> {
        Ok(self
            .client
            .header()
            .subscribe()
            .map(|res| res.map(CelestiaHeader::from).map_err(|e| e.into()))
            .boxed())
    }

    pub async fn fetch_block_parts(
        &self,
        height: u64,
        batch_namespace: Namespace,
        proof_namespace: Namespace,
    ) -> anyhow::Result<(CelestiaHeader, NamespaceData, NamespaceData)> {
        let start_get_block = std::time::Instant::now();
        let header_future = run_maybe_retryable_async_fn_with_retries(
            self.backoff,
            || self.get_header_at(height),
            "get_block_header_at",
        );
        let rollup_batch_rows_future = run_maybe_retryable_async_fn_with_retries(
            self.backoff,
            || self.get_namespace_data_at(height, batch_namespace, RollupNamespace::Batch),
            "get_rollup_batch_namespace",
        );
        let rollup_proof_rows_future = run_maybe_retryable_async_fn_with_retries(
            self.backoff,
            || self.get_namespace_data_at(height, proof_namespace, RollupNamespace::Proof),
            "get_rollup_proof_namespace",
        );
        let (header, batch_rows, proof_rows) = tokio::try_join!(
            header_future,
            rollup_batch_rows_future,
            rollup_proof_rows_future,
        )?;
        let futures_time = start_get_block.elapsed();
        let build_relevant_data_start = std::time::Instant::now();
        let batch_ns_metrics = NamespaceDataMetrics::new(&batch_rows);
        let proof_ns_metrics = NamespaceDataMetrics::new(&proof_rows);
        let build_relevant_data = build_relevant_data_start.elapsed();
        let total_time = start_get_block.elapsed();
        let square_width = header.dah.square_width() as u16;
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(GetBlockMeasurement {
                height,
                square_width,
                futures_time,
                build_relevant_data,
                batch_ns_metrics,
                proof_ns_metrics,
                total_time,
            });
        });
        Ok((header, batch_rows, proof_rows))
    }
}

pub fn apply_verify_on_fetch(
    mode: VerifyOnFetchMode,
    height: u64,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    match mode {
        VerifyOnFetchMode::Off => Ok(()),
        VerifyOnFetchMode::LogError => {
            if let Err(error) = result {
                tracing::error!(
                    height,
                    ?error,
                    "Celestia block integrity verification failed; continuing because \
                     verify_on_fetch_mode is `log_error`"
                );
            }
            Ok(())
        }
        VerifyOnFetchMode::ReturnError => result.with_context(|| {
            format!("Celestia block integrity verification failed at height {height}")
        }),
    }
}

pub fn flatten_timeout<T>(
    response: Result<Result<T, celestia_client::Error>, tokio::time::error::Elapsed>,
) -> Result<T, MaybeRetryable<anyhow::Error>> {
    match response {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(e)) => Err(MaybeRetryable::Transient(e.into())),
        Err(_) => Err(MaybeRetryable::Transient(anyhow::anyhow!("await timeout"))),
    }
}

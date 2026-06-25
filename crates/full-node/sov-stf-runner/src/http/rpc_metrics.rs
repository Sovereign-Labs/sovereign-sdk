//! jsonrpsee middleware that records every JSON-RPC call — over both HTTP and
//! WebSocket — into an in-process [`RpcStatsAggregator`], which aggregates
//! per-method statistics and periodically flushes them to the metrics server.
//! Calls whose future is dropped before completion (client disconnect or
//! timeout) are recorded as cancelled via a drop guard. Slow calls
//! additionally emit an individual metric point (capped per window) and a
//! debug-level log line that includes the request parameters; parameters are
//! deliberately kept out of the emitted metrics.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonrpsee::server::middleware::rpc::{
    Batch, MethodResponse, Notification, Request, RpcServiceT,
};
use sov_metrics::{RpcStatsAggregator, UNKNOWN_METHOD};

/// Request parameters are truncated to this many bytes in slow-call log lines.
const MAX_LOGGED_PARAMS_BYTES: usize = 4096;

/// A [`tower_layer::Layer`] producing [`RpcMetricsService`], for use with
/// `jsonrpsee`'s `RpcServiceBuilder`.
#[derive(Clone)]
pub(crate) struct RpcMetricsLayer {
    aggregator: Arc<RpcStatsAggregator>,
    is_ws: bool,
}

impl RpcMetricsLayer {
    pub(crate) fn new(aggregator: Arc<RpcStatsAggregator>, is_ws: bool) -> Self {
        Self { aggregator, is_ws }
    }
}

impl<S> tower_layer::Layer<S> for RpcMetricsLayer {
    type Service = RpcMetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcMetricsService {
            inner,
            aggregator: self.aggregator.clone(),
            is_ws: self.is_ws,
        }
    }
}

/// The middleware service; see the [module documentation](self).
#[derive(Clone)]
pub(crate) struct RpcMetricsService<S> {
    inner: S,
    aggregator: Arc<RpcStatsAggregator>,
    is_ws: bool,
}

/// Which series a cancellation is recorded under.
enum GuardTarget {
    Method(&'static str),
    Batch,
}

/// Records the call as cancelled when dropped before [`Self::complete`] —
/// which happens exactly when the in-flight request future is dropped (client
/// disconnect or timeout). Without this, the slowest calls — the ones clients
/// give up on — would be invisible to the metrics, making an overload look
/// like a traffic drop instead of a latency spike.
///
/// Recording from `Drop` is safe here because the aggregator's recording path
/// is a non-blocking `try_send` plus atomics; it cannot block, deadlock, or
/// panic.
struct CancellationGuard {
    aggregator: Arc<RpcStatsAggregator>,
    target: GuardTarget,
    is_ws: bool,
    start: Instant,
    armed: bool,
}

impl CancellationGuard {
    /// Starts timing. Construct before the request future is created so that
    /// a future dropped without ever being polled is still observed.
    fn new(aggregator: Arc<RpcStatsAggregator>, target: GuardTarget, is_ws: bool) -> Self {
        Self {
            aggregator,
            target,
            is_ws,
            start: Instant::now(),
            armed: true,
        }
    }

    /// Disarms the guard and returns the elapsed time since construction.
    /// Call on normal completion.
    fn complete(mut self) -> Duration {
        self.armed = false;
        self.start.elapsed()
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let elapsed = self.start.elapsed();
        match self.target {
            GuardTarget::Method(method) => {
                self.aggregator
                    .record_cancelled(method, self.is_ws, elapsed);
            }
            GuardTarget::Batch => {
                self.aggregator.record_cancelled_batch(self.is_ws, elapsed);
            }
        }
    }
}

impl<S> RpcServiceT for RpcMetricsService<S>
where
    S: RpcServiceT<
            MethodResponse = MethodResponse,
            BatchResponse = MethodResponse,
            NotificationResponse = MethodResponse,
        > + Send
        + Sync
        + Clone
        + 'static,
{
    type MethodResponse = MethodResponse;
    type BatchResponse = MethodResponse;
    type NotificationResponse = MethodResponse;

    fn call<'a>(
        &self,
        request: Request<'a>,
    ) -> impl std::future::Future<Output = Self::MethodResponse> + Send + 'a {
        let inner = self.inner.clone();
        let aggregator = self.aggregator.clone();
        let is_ws = self.is_ws;
        // Resolving up front yields a `&'static str` that outlives `request`,
        // which is consumed by the inner service before we record the call.
        let method = aggregator.resolve_method(request.method_name());
        let debug_enabled = tracing::enabled!(tracing::Level::DEBUG);
        // The folded `method` tag protects InfluxDB cardinality, but in a log
        // line it would hide which unregistered method was slow — keep the
        // wire name, allocating only when it differs from the tag and the log
        // below could actually fire.
        let raw_method_for_slow_log =
            (debug_enabled && method == UNKNOWN_METHOD).then(|| request.method_name().to_owned());
        // Parameters are consumed with the request, so capture them now. The
        // `Cow` clone is a pointer copy on the (usual) borrowed path, keeping
        // the common case allocation-free; truncation happens at log time.
        let params_for_slow_log = if debug_enabled {
            request.params.clone()
        } else {
            None
        };
        let guard = CancellationGuard::new(aggregator.clone(), GuardTarget::Method(method), is_ws);

        async move {
            let response = inner.call(request).await;
            let duration = guard.complete();
            let error_code = response.as_error_code();
            let recorded = aggregator.record_call_resolved(method, is_ws, duration, error_code);
            if recorded.is_slow() {
                tracing::debug!(
                    method,
                    raw_method = raw_method_for_slow_log.as_deref(),
                    is_ws,
                    duration_ms = duration.as_millis() as u64,
                    error_code,
                    params = params_for_slow_log
                        .as_ref()
                        .map(|params| truncate_utf8(params.get(), MAX_LOGGED_PARAMS_BYTES)),
                    "Slow RPC call"
                );
            }
            response
        }
    }

    fn batch<'a>(
        &self,
        batch: Batch<'a>,
    ) -> impl std::future::Future<Output = Self::BatchResponse> + Send + 'a {
        let inner = self.inner.clone();
        let aggregator = self.aggregator.clone();
        let is_ws = self.is_ws;

        // The inner service dispatches batch entries internally, so this is
        // the only place per-entry methods are visible. Entries are counted
        // here (untimed), while the whole batch is timed below under the
        // `batch` pseudo-method.
        let mut entry_count: u64 = 0;
        let mut malformed_entries: u64 = 0;
        let mut entry_methods = Vec::new();
        for entry in batch.iter() {
            entry_count += 1;
            match entry {
                Ok(entry) => entry_methods.push(aggregator.resolve_method(entry.method_name())),
                // Malformed entries carry no method name; they are counted
                // here and appear as error objects in the response body,
                // where `count_failed_entries` picks them up.
                Err(_) => malformed_entries += 1,
            }
        }
        // The resolved names are needed again only by the slow-batch log
        // below; clone them only when that log could actually fire.
        let methods_for_slow_log =
            tracing::enabled!(tracing::Level::DEBUG).then(|| entry_methods.clone());
        aggregator.record_batch_entries(entry_methods, is_ws);
        let guard = CancellationGuard::new(aggregator.clone(), GuardTarget::Batch, is_ws);

        async move {
            let response = inner.batch(batch).await;
            let duration = guard.complete();
            let error_code = response.as_error_code();
            // jsonrpsee marks the batch response as successful regardless of
            // per-entry failures (`error_code` only reflects whole-batch
            // rejections, e.g. an oversized batch), so per-entry errors must
            // be counted from the response body.
            let failed_entries = count_failed_entries(response.as_json().get());
            let recorded = aggregator.record_batch(is_ws, duration, error_code, failed_entries);
            if recorded.is_slow() {
                tracing::debug!(
                    is_ws,
                    duration_ms = duration.as_millis() as u64,
                    error_code,
                    entry_count,
                    malformed_entries,
                    failed_entries,
                    methods = ?methods_for_slow_log.as_deref(),
                    "Slow RPC batch"
                );
            }
            response
        }
    }

    fn notification<'a>(
        &self,
        notification: Notification<'a>,
    ) -> impl std::future::Future<Output = Self::NotificationResponse> + Send + 'a {
        let inner = self.inner.clone();
        let aggregator = self.aggregator.clone();
        let is_ws = self.is_ws;
        let method = aggregator.resolve_method(notification.method_name());

        async move {
            let response = inner.notification(notification).await;
            // Notifications have no response payload, so they cannot error
            // and there is nothing meaningful to time; they are counted in
            // their own dedicated counter.
            aggregator.record_notification(method, is_ws);
            response
        }
    }
}

/// Helper struct for counting failed entries in a batch response.
#[derive(serde::Deserialize)]
struct EntryProbe {
    error: Option<serde::de::IgnoredAny>,
}

/// Counts the entries of a serialized JSON-RPC batch response that carry an
/// `error` member. Returns 0 when the body is not a JSON array (e.g. a
/// whole-batch rejection, which is covered by the response's error code
/// instead).
///
/// This is a single pass over the serialized response
/// but the cost is still proportional to the response size and is paid
/// for every batch, including fast ones: the `failed_entries` counter is part
/// of every window's aggregate, and jsonrpsee's middleware only exposes the
/// batch response as a serialized whole, so re-parsing it is the only way to
/// count per-entry failures. Large batched queries (e.g. `eth_getLogs`) make
/// this the most expensive step of recording a batch.
fn count_failed_entries(batch_response_json: &str) -> u64 {
    serde_json::from_str::<Vec<EntryProbe>>(batch_response_json)
        .map(|entries| entries.iter().filter(|e| e.error.is_some()).count() as u64)
        .unwrap_or(0)
}

/// Truncates `s` to at most `max_bytes`, backing up to a UTF-8 char boundary.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonrpsee::core::server::ResponsePayload;
    use jsonrpsee::types::Id;
    use sov_metrics::{MonitoringConfig, RpcAggregationConfig, TelegrafSocketConfig};
    use tower_layer::Layer;

    /// Inner service stub that succeeds for `eth_getLogs` and errors for
    /// everything else.
    #[derive(Clone)]
    struct StubService;

    impl RpcServiceT for StubService {
        type MethodResponse = MethodResponse;
        type BatchResponse = MethodResponse;
        type NotificationResponse = MethodResponse;

        // Match the signature of RpcService exactly rather than using the new sugar.
        #[allow(clippy::manual_async_fn)]
        fn call<'a>(
            &self,
            request: Request<'a>,
        ) -> impl std::future::Future<Output = MethodResponse> + Send + 'a {
            async move {
                if request.method_name() == "eth_getLogs" {
                    MethodResponse::response(
                        request.id.clone(),
                        ResponsePayload::success("ok"),
                        usize::MAX,
                    )
                } else {
                    MethodResponse::error(
                        request.id.clone(),
                        jsonrpsee::types::ErrorObject::owned(-32601, "nope", None::<()>),
                    )
                }
            }
        }

        // Match the signature of RpcService exactly rather than using the new sugar.
        #[allow(clippy::manual_async_fn)]
        fn batch<'a>(
            &self,
            _batch: Batch<'a>,
        ) -> impl std::future::Future<Output = MethodResponse> + Send + 'a {
            async move {
                MethodResponse::response(Id::Number(1), ResponsePayload::success("ok"), usize::MAX)
            }
        }

        // Match the signature of RpcService exactly rather than using the new sugar.
        #[allow(clippy::manual_async_fn)]
        fn notification<'a>(
            &self,
            _notification: Notification<'a>,
        ) -> impl std::future::Future<Output = MethodResponse> + Send + 'a {
            async move { MethodResponse::notification() }
        }
    }

    /// Inner service stub whose futures never resolve, for exercising the
    /// cancellation path.
    #[derive(Clone)]
    struct PendingService;

    impl RpcServiceT for PendingService {
        type MethodResponse = MethodResponse;
        type BatchResponse = MethodResponse;
        type NotificationResponse = MethodResponse;

        fn call<'a>(
            &self,
            _request: Request<'a>,
        ) -> impl std::future::Future<Output = MethodResponse> + Send + 'a {
            std::future::pending()
        }

        fn batch<'a>(
            &self,
            _batch: Batch<'a>,
        ) -> impl std::future::Future<Output = MethodResponse> + Send + 'a {
            std::future::pending()
        }

        fn notification<'a>(
            &self,
            _notification: Notification<'a>,
        ) -> impl std::future::Future<Output = MethodResponse> + Send + 'a {
            std::future::pending()
        }
    }

    fn request(method: &'static str) -> Request<'static> {
        Request::owned(method.to_owned(), None, Id::Number(1))
    }

    #[test]
    fn count_failed_entries_counts_error_objects() {
        let body = r#"[
            {"jsonrpc":"2.0","result":"ok","id":1},
            {"jsonrpc":"2.0","error":{"code":-32601,"message":"nope"},"id":2},
            {"jsonrpc":"2.0","error":{"code":-32602,"message":"bad params"},"id":3}
        ]"#;
        assert_eq!(count_failed_entries(body), 2);
    }

    #[test]
    fn count_failed_entries_returns_zero_for_non_array_body() {
        let body = r#"{"jsonrpc":"2.0","error":{"code":-32011,"message":"too big"},"id":null}"#;
        assert_eq!(count_failed_entries(body), 0);
    }

    /// One end-to-end test instead of one per concern: the metrics tracker is
    /// a process-wide singleton bound to one UDP socket, so all assertions
    /// about published points must share a single test.
    #[tokio::test(flavor = "multi_thread")]
    async fn middleware_aggregates_calls_through_metrics_pipeline() -> anyhow::Result<()> {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
        let monitoring_config = MonitoringConfig {
            telegraf_address: TelegrafSocketConfig::udp(socket.local_addr()?),
            // Publish each metric immediately instead of buffering.
            max_datagram_size: Some(1),
            max_pending_metrics: None,
            tokio_runtime_metrics_interval_millis: 500,
            rpc_aggregation: RpcAggregationConfig::standard(),
        };
        let secondary_shutdown_controller =
            sov_shutdown::SecondaryShutdownController::new();
        sov_metrics::init_metrics_tracker(&monitoring_config, &secondary_shutdown_controller);

        let aggregator = Arc::new(RpcStatsAggregator::new(
            RpcAggregationConfig::standard(),
            ["eth_getLogs"],
        ));
        let http = RpcMetricsLayer::new(aggregator.clone(), false).layer(StubService);
        let ws = RpcMetricsLayer::new(aggregator.clone(), true).layer(StubService);

        http.call(request("eth_getLogs")).await;
        ws.call(request("made_up_method")).await;

        // Dropping an in-flight future must be recorded as a cancellation.
        // Uses the (eth_getLogs, ws) pair, which no other call in this test
        // touches, so its point is distinguishable below.
        let pending_ws = RpcMetricsLayer::new(aggregator.clone(), true).layer(PendingService);
        let in_flight = pending_ws.call(request("eth_getLogs"));
        drop(in_flight);

        // Drive the consumer: shutting it down immediately makes it drain all
        // recorded events and perform a final flush.
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let flush_loop = tokio::spawn(aggregator.clone().run_flush_loop(async move {
            // The sender is kept alive until after `send`, so this cannot err.
            let _ = shutdown_rx.await;
        }));
        shutdown_tx
            .send(())
            .expect("flush loop holds the receiver until it completes");
        flush_loop.await?;

        let expected_http =
            "sov_rollup_rpc_aggregated,method=eth_getLogs,is_ws=false calls=1,errors=0,";
        let expected_ws = "sov_rollup_rpc_aggregated,method=unknown,is_ws=true calls=1,errors=1,";
        let expected_cancelled = "sov_rollup_rpc_aggregated,method=eth_getLogs,is_ws=true \
                                  calls=0,errors=0,batched_entries=0,failed_entries=0,cancelled=1,";
        let (mut seen_http, mut seen_ws, mut seen_cancelled) = (false, false, false);
        let mut received = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut buffer = [0u8; 65536];
        while !(seen_http && seen_ws && seen_cancelled) {
            // Other tests in this binary may publish to the same global
            // tracker, so scan datagrams instead of asserting on the first.
            let read = tokio::time::timeout_at(deadline, socket.recv(&mut buffer))
                .await
                .unwrap_or_else(|_| panic!("expected points not received: {received:?}"))?;
            let datagram = String::from_utf8_lossy(&buffer[..read]).to_string();
            seen_http |= datagram.contains(expected_http);
            seen_ws |= datagram.contains(expected_ws);
            seen_cancelled |= datagram.contains(expected_cancelled);
            received.push(datagram);
        }
        Ok(())
    }
}

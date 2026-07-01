use crate::http::id_provider::HexIdProvider;
use crate::CorsConfiguration;
use axum::body::HttpBody;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{ConnectInfo, MatchedPath, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::serve::ListenerExt;
use axum::ServiceExt;
use jsonrpsee::server::middleware::rpc::RpcServiceBuilder;
use jsonrpsee::server::{
    stop_channel, Methods, ServerBuilder, ServerConfig, ServerHandle, StopHandle, TowerService,
};
use jsonrpsee::types::{ErrorCode, ErrorObject};
use jsonrpsee::RpcModule;
use sov_metrics::{track_metrics, HttpMetrics, RpcAggregationConfig, RpcStatsAggregator};
use sov_shutdown::{BackgroundHandle, RunnerShutdownController};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::BoxError;
use tower_http::cors::CorsLayer;
use tower_http::normalize_path::NormalizePathLayer;
use tower_layer::{Identity, Layer, Stack};
mod id_provider;
mod rpc_metrics;
use rpc_metrics::RpcMetricsLayer;
use sov_rest_utils::get_client_ip;
use sov_rest_utils::GetIPResult;

// Middleware to inject SocketAddr from axum's ConnectInfo into the request extensions
// so that jsonrpsee RPC handlers can access it via the Extensions parameter
#[derive(Clone)]
struct InjectSocketAddrLayer;

impl<S> Layer<S> for InjectSocketAddrLayer {
    type Service = InjectSocketAddrService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        InjectSocketAddrService { inner }
    }
}

#[derive(Clone)]
struct InjectSocketAddrService<S> {
    inner: S,
}

impl<S, B> tower::Service<axum::http::Request<B>> for InjectSocketAddrService<S>
where
    S: tower::Service<axum::http::Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: axum::http::Request<B>) -> Self::Future {
        // Extract SocketAddr from axum's ConnectInfo and insert it directly
        // into extensions so jsonrpsee can access it
        let headers = req.headers();
        let connect_info = req.extensions().get::<ConnectInfo<SocketAddr>>();

        let maybe_ip = get_client_ip(headers.clone(), connect_info);
        req.extensions_mut().insert(GetIPResult {
            maybe_ip: Arc::new(maybe_ip),
        });

        self.inner.call(req)
    }
}

pub struct HttpServerStart {
    pub http_server_handle: BackgroundHandle<anyhow::Result<()>>,
    pub rpc_metrics_flush_handle: BackgroundHandle<anyhow::Result<()>>,
}

/// Starts the HTTP server and the RPC metrics flush task, returning both
/// join handles so callers can observe either task's termination.
pub(crate) async fn start_http_server(
    axum_listener: TcpListener,
    router: axum::Router<()>,
    methods: RpcModule<()>,
    shutdown: RunnerShutdownController,
    cors_configuration: CorsConfiguration,
    rpc_aggregation: RpcAggregationConfig,
) -> anyhow::Result<HttpServerStart> {
    let rest_address = axum_listener.local_addr()?;
    let (rpc_router, server_handle, aggregator) =
        rpc_module_to_router(methods, cors_configuration, rpc_aggregation);

    // Drives the aggregator until the RPC server has fully stopped; the final
    // flush then captures calls that completed during graceful shutdown.
    let server_stopped = server_handle.clone();
    let flush_handle = BackgroundHandle::spawn("rpc-metrics-flush", async move {
        aggregator
            .run_flush_loop(async move { server_stopped.stopped().await })
            .await;
        anyhow::Ok(())
    });

    let handle = BackgroundHandle::spawn("http-server", async move {
        tracing::info!(%rest_address, "Starting HTTP server");
        let mut router = router.layer(axum::middleware::from_fn(measure_time));
        if let CorsConfiguration::Permissive = cors_configuration {
            router = router.layer(CorsLayer::permissive());
        }
        let router = router.nest("/rpc", rpc_router);
        let router = NormalizePathLayer::trim_trailing_slash().layer(router);

        // TODO: Is there a way to have max_connections and other params for axum::serve?
        let axum_listener = axum_listener.tap_io(|tcp| {
            let _ = tcp.set_nodelay(true);
        });
        let result = axum::serve(
            axum_listener,
            ServiceExt::<axum::extract::Request>::into_make_service_with_connect_info::<SocketAddr>(
                router,
            ),
        )
        .with_graceful_shutdown(async move {
            shutdown.wait_for_shutdown().await.ok();
        })
        .await
        .map_err(|e| anyhow::anyhow!(e));

        if let Err(error) = server_handle.stop() {
            // It could've been stopped already by axum shutdown.
            tracing::trace!(%error, "Failed to stop RPC server");
        };
        // Wait till it actually stopped
        server_handle.stopped().await;

        result
    });
    Ok(HttpServerStart {
        http_server_handle: handle,
        rpc_metrics_flush_handle: flush_handle,
    })
}

/// Build [`axum::Router`] from [`jsonrpsee::RpcModule`] with support of websocket.
///
/// Also returns the per-call statistics aggregator wired into the RPC
/// services. The caller must drive [`RpcStatsAggregator::run_flush_loop`]
/// (typically via `tokio::spawn`, observing the task) for aggregated metrics
/// to be emitted; without it, recorded events are silently discarded once the
/// aggregator's internal channel fills up, channel-overflow warnings are never
/// logged, and individual slow-call points — which initially work, since they
/// are emitted directly from the recording path — stop permanently once the
/// per-window cap fills, because the cap is only reset at flush time.
pub fn rpc_module_to_router(
    methods: RpcModule<()>,
    cors_config: CorsConfiguration,
    rpc_aggregation: RpcAggregationConfig,
) -> (axum::Router, ServerHandle, Arc<RpcStatsAggregator>) {
    let (stop_handle, server_handle) = stop_channel();
    let cors_layer = match cors_config {
        CorsConfiguration::Permissive => CorsLayer::permissive(),
        // New does not set any CORS headers
        CorsConfiguration::Restrictive => CorsLayer::new(),
    };

    // Per-call metrics are aggregated in-process and flushed periodically; see
    // `sov_metrics::RpcStatsAggregator`. The registered method names seed the
    // cardinality guard for the `method` tag.
    let methods: Methods = methods.into();
    let aggregator = Arc::new(RpcStatsAggregator::new(
        rpc_aggregation,
        methods.method_names(),
    ));

    let ws_service = ws_service(
        methods.clone(),
        stop_handle.clone(),
        RpcMetricsLayer::new(aggregator.clone(), true),
    );
    let http_service = http_service(
        methods,
        stop_handle,
        RpcMetricsLayer::new(aggregator.clone(), false),
    );
    let error_layer = HandleErrorLayer::new(|error: BoxError| async move {
        let error = ErrorObject::owned(
            ErrorCode::InternalError.code(),
            error.to_string(),
            None::<()>,
        );
        (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    });

    let http_service = error_layer.layer(http_service);
    let ws_service = error_layer.layer(ws_service);

    // Wrap services with the SocketAddr injection layer
    let inject_addr_layer = InjectSocketAddrLayer;
    let http_service = inject_addr_layer.layer(http_service);
    let ws_service = inject_addr_layer.layer(ws_service);

    let router = axum::routing::get_service(ws_service)
        .post_service(http_service)
        .layer(cors_layer);
    (
        axum::Router::new().route("/", router),
        server_handle,
        aggregator,
    )
}

fn http_service(
    methods: Methods,
    stop_handle: StopHandle,
    metrics: RpcMetricsLayer,
) -> TowerService<Stack<RpcMetricsLayer, Identity>, Identity> {
    // TODO: Into config.toml
    let config = ServerConfig::builder()
        .http_only()
        .max_connections(10_000)
        .build();

    ServerBuilder::with_config(config)
        .set_rpc_middleware(RpcServiceBuilder::new().layer(metrics))
        .to_service_builder()
        .build(methods, stop_handle)
}

fn ws_service(
    methods: Methods,
    stop_handle: StopHandle,
    metrics: RpcMetricsLayer,
) -> TowerService<Stack<RpcMetricsLayer, Identity>, Identity> {
    // TODO: Into config.toml
    let config = ServerConfig::builder()
        .set_id_provider(HexIdProvider::default())
        .ws_only()
        .max_connections(10_000)
        .max_subscriptions_per_connection(100)
        .build();
    ServerBuilder::with_config(config)
        .set_rpc_middleware(RpcServiceBuilder::new().layer(metrics))
        .to_service_builder()
        .build(methods, stop_handle)
}

async fn measure_time(
    matched_path: Option<MatchedPath>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    let method = req.method().clone();
    let start = std::time::Instant::now();

    let response = next.run(req).await;
    let duration = start.elapsed();

    // Skip metrics for unmatched routes (404s) to avoid cardinality explosion
    // from arbitrary paths hitting the server.
    if let Some(matched_path) = matched_path {
        let body = response.body();
        let status = response.status();
        let size_hint = body.size_hint();
        let exact_or_lower = size_hint.exact().unwrap_or_else(|| size_hint.lower());

        track_metrics(|tracker| {
            let point = HttpMetrics {
                request_method: method,
                request_path: matched_path.as_str().to_owned(),
                response_status: status,
                response_body_size: exact_or_lower,
                handler_processing_time: duration,
                is_ws: false,
            };
            tracker.submit_known_metric(point);
        });
    }

    response
}

#[cfg(test)]
mod tests {
    use futures_util::sink::SinkExt;
    use futures_util::stream::StreamExt;
    use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
    use jsonrpsee::core::JsonRawValue;
    use jsonrpsee::ws_client::WsClientBuilder;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

    use super::*;

    const RPC_READ_METHOD: &str = "test_hello";
    const RPC_SUBSCRIBE_METHOD: &str = "subscribe_numbers";
    const RPC_UNSUBSCRIBE_METHOD: &str = "unsubscribe_numbers";

    fn build_test_json_rpc() -> RpcModule<()> {
        let mut module = RpcModule::new(());

        module
            .register_method(RPC_READ_METHOD, |_, _, _| {
                tracing::info!("Regular method '{}' called", RPC_READ_METHOD);
                "hi"
            })
            .unwrap();

        module
            .register_subscription(
                RPC_SUBSCRIBE_METHOD,
                RPC_SUBSCRIBE_METHOD,
                RPC_UNSUBSCRIBE_METHOD,
                |params, pending, _tx, _| {
                    tracing::info!(
                        "Subscription '{}' requested with params: {:?}",
                        RPC_SUBSCRIBE_METHOD,
                        params
                    );

                    async move {
                        tracing::info!("Starting subscription handler execution");
                        match pending.accept().await {
                            Ok(sub) => {
                                tracing::info!("Subscription accepted successfully");
                                let _method = sub.method_name();
                                let _sub_id = sub.subscription_id();
                                for i in 0..usize::MAX {
                                    let r = JsonRawValue::from_string(i.to_string()).unwrap();
                                    if sub.send(r).await.is_err() {
                                        break;
                                    };
                                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                                }
                                tracing::info!("Completed sending all notifications");
                            }
                            Err(e) => tracing::error!(error = %e, "Failed to accept subscription"),
                        }

                        Ok(())
                    }
                },
            )
            .unwrap();

        module
    }

    fn build_test_axum_router() -> axum::Router<()> {
        axum::Router::new().route("/", axum::routing::get(|| async { "hi" }))
    }

    // Returns the shutdown controller used to stop the server.
    async fn build_and_start_test_server() -> (SocketAddr, RunnerShutdownController) {
        let methods = build_test_json_rpc();
        let axum_router = build_test_axum_router();
        let shutdown = RunnerShutdownController::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _ = start_http_server(
            listener,
            axum_router,
            methods,
            shutdown.clone(),
            CorsConfiguration::Restrictive,
            RpcAggregationConfig::standard(),
        )
        .await
        .unwrap();

        (addr, shutdown)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_request_response() -> anyhow::Result<()> {
        let (addr, shutdown) = build_and_start_test_server().await;

        let ws_client = WsClientBuilder::default()
            .build(&format!("ws://{addr}/rpc"))
            .await?;

        for _ in 0..10 {
            let response = ws_client
                .request::<String, [u8; 0]>(RPC_READ_METHOD, [])
                .await?;

            assert_eq!(response, "hi");
        }
        shutdown.shutdown();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_subscription() -> anyhow::Result<()> {
        let (addr, shutdown) = build_and_start_test_server().await;

        let ws_client = WsClientBuilder::default()
            .build(&format!("ws://{addr}/rpc"))
            .await?;

        let mut subscription = ws_client
            .subscribe::<u64, [u8; 0]>(RPC_SUBSCRIBE_METHOD, [], RPC_UNSUBSCRIBE_METHOD)
            .await?;

        let numbers = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut numbers: Vec<u64> = vec![];
            for _ in 0..10 {
                let number: u64 = subscription.next().await.unwrap().unwrap();
                numbers.push(number);
            }
            numbers
        })
        .await?;
        subscription.unsubscribe().await?;
        assert_eq!(numbers, (0..10).collect::<Vec<u64>>());

        shutdown.shutdown();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_binary_frames() -> anyhow::Result<()> {
        let (addr, shutdown) = build_and_start_test_server().await;

        // Connect using raw tungstenite client to send binary frames
        let (ws_stream, _) = connect_async(format!("ws://{addr}/rpc"))
            .await
            .expect("Failed to connect");
        let (mut write, mut read) = ws_stream.split();

        // Send a JSON-RPC request as binary frame
        let request = r#"{"jsonrpc":"2.0","method":"test_hello","params":[],"id":1}"#;
        write
            .send(TungsteniteMessage::Binary(
                request.as_bytes().to_vec().into(),
            ))
            .await?;

        // jsonrpsee accepts binary frames but responds in text format: https://github.com/paritytech/jsonrpsee/pull/374
        let response = read.next().await.expect("No response")?;
        let TungsteniteMessage::Text(response) = response else {
            panic!("Expected text response, got: {response:?}");
        };
        assert!(response.contains("\"result\":\"hi\""));

        shutdown.shutdown();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ws_duplex() -> anyhow::Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(3), test_ws_duplex_inner()).await?
    }

    async fn test_ws_duplex_inner() -> anyhow::Result<()> {
        let (addr, shutdown) = build_and_start_test_server().await;

        let ws_client = WsClientBuilder::default()
            .build(&format!("ws://{addr}/rpc"))
            .await?;

        let mut subscription = ws_client
            .subscribe::<u64, [u8; 0]>(RPC_SUBSCRIBE_METHOD, [], RPC_UNSUBSCRIBE_METHOD)
            .await?;

        let number: u64 = subscription.next().await.unwrap().unwrap();
        assert_eq!(number, 0);

        let response = ws_client
            .request::<String, [u8; 0]>(RPC_READ_METHOD, [])
            .await?;
        assert_eq!(response, "hi");

        let number: u64 = subscription.next().await.unwrap().unwrap();
        assert_eq!(number, 1);

        let response = ws_client
            .request::<String, [u8; 0]>(RPC_READ_METHOD, [])
            .await?;
        assert_eq!(response, "hi");

        shutdown.shutdown();

        Ok(())
    }
}

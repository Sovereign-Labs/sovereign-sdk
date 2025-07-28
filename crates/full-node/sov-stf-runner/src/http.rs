use std::net::SocketAddr;

use axum::body::HttpBody;
use axum::error_handling::HandleErrorLayer;
use axum::extract::ws::{Message, WebSocket};
use axum::http::StatusCode;
use axum::ServiceExt;
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use jsonrpsee::server::ServerConfig;
use jsonrpsee::RpcModule;
use tokio::sync::watch;
use tower::BoxError;
use tower_http::cors::CorsLayer;
use tower_http::normalize_path::NormalizePathLayer;
use tower_layer::Layer;

use crate::CorsConfiguration;

pub(crate) async fn start_http_server(
    listen_address_http: &SocketAddr,
    router: axum::Router<()>,
    methods: RpcModule<()>,
    mut shutdown_receiver: watch::Receiver<()>,
    cors_configuration: CorsConfiguration,
) -> anyhow::Result<(tokio::task::JoinHandle<anyhow::Result<()>>, SocketAddr)> {
    let listener = tokio::net::TcpListener::bind(listen_address_http).await?;
    let rest_address = listener.local_addr()?;

    let (rpc_router, server_handle) = rpc_module_to_router(methods, cors_configuration);

    let handle = tokio::spawn(async move {
        tracing::info!(%rest_address, "Starting HTTP server");
        let mut router = router.layer(axum::middleware::from_fn(measure_time));
        if let CorsConfiguration::Permissive = cors_configuration {
            router = router.layer(CorsLayer::permissive());
        }
        let router = router.nest("/rpc", rpc_router);
        let router = NormalizePathLayer::trim_trailing_slash().layer(router);

        // TODO: Is there a way to have max_connections and other params for axum::serve?
        let result = axum::serve(
            listener,
            ServiceExt::<axum::extract::Request>::into_make_service(router),
        )
        .with_graceful_shutdown(async move {
            shutdown_receiver.changed().await.ok();
        })
        .tcp_nodelay(true)
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

    Ok((handle, rest_address))
}

/// Build [`axum::Router`] from [`jsonrpsee::RpcModule`] with support of websocket.
pub fn rpc_module_to_router(
    methods: RpcModule<()>,
    cors_config: CorsConfiguration,
) -> (axum::Router, jsonrpsee::server::ServerHandle) {
    let (stop_handle, server_handle) = jsonrpsee::server::stop_channel();

    // TODO: Into config.toml
    let config = ServerConfig::builder()
        .max_connections(10_000)
        .max_subscriptions_per_connection(100)
        .build();
    let rpc_service = jsonrpsee::server::ServerBuilder::with_config(config)
        .to_service_builder()
        .build(methods.clone(), stop_handle);

    let rpc_service = tower::ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|error: BoxError| async move {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                jsonrpsee::types::error::ErrorObject::owned(
                    jsonrpsee::types::error::ErrorCode::InternalError.code(),
                    error.to_string(),
                    None::<String>,
                )
                .to_string(),
            )
        }))
        .service(rpc_service);

    let cors_layer = match cors_config {
        CorsConfiguration::Permissive => CorsLayer::permissive(),
        // New does not set any CORS headers
        CorsConfiguration::Restrictive => CorsLayer::new(),
    };

    (
        axum::Router::new().route(
            "/",
            axum::routing::get(move |ws_upgrade| ws_rpc_handler(ws_upgrade, methods.clone()))
                .post_service(rpc_service)
                .layer(cors_layer),
        ),
        server_handle,
    )
}

async fn measure_time(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl axum::response::IntoResponse {
    let method = req.method().clone();
    let uri = req.uri().clone();

    let start = std::time::Instant::now();

    let response = next.run(req).await;
    let duration = start.elapsed();

    let body = response.body();
    let status = response.status();
    let size_hint = body.size_hint();
    let exact_or_lower = size_hint.exact().unwrap_or_else(|| size_hint.lower());

    sov_metrics::track_metrics(|tracker| {
        let point = sov_metrics::HttpMetrics {
            request_method: method,
            request_uri: uri,
            response_status: status,
            response_body_size: exact_or_lower,
            handler_processing_time: duration,
        };
        tracker.submit(point);
    });

    response
}

async fn ws_rpc_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    rpc_methods: RpcModule<()>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| async move {
        handle_socket(socket, rpc_methods).await;
    })
}

// To support duplex communication socket is split into 2 streams,
// each stream is processed in their own task
// 1. Reader task receives requests from websocket and pushes appropriate responses into the mpsc channel to the writer task.
//    In the case of subscriptions, the reader task clones `tokio::sync::mpsc::Sender` and spawns another task,
//    where subscription responses are piped to the writer task.
// 2. Writer task listens to [`tokio::sync::mpsc::Receiver`] and writes responses to websocket.
async fn handle_socket(socket: WebSocket, rpc_methods: RpcModule<()>) {
    let (sender, receiver) = socket.split();

    let (socket_requests, socket_responses) = tokio::sync::mpsc::channel(10);

    tokio::spawn(handle_socket_write(socket_responses, sender));
    tokio::spawn(handle_socket_read(receiver, socket_requests, rpc_methods));
}

async fn handle_socket_read(
    mut socket_requests: futures_util::stream::SplitStream<WebSocket>,
    socket_responses: tokio::sync::mpsc::Sender<Message>,
    rpc_methods: RpcModule<()>,
) {
    while let Some(Ok(msg)) = socket_requests.next().await {
        tracing::trace!(message = ?msg, "Message received from websocket");
        match msg {
            Message::Text(text) => {
                // Buffer size picked up from `jsonrpsee` crate examples
                match rpc_methods.raw_json_request(&text, 1).await {
                    Ok((rpc_response, mut receiver)) => {
                        tracing::trace!("RPC request processed successfully: {}", rpc_response);
                        if socket_responses
                            .send(Message::Text(rpc_response.to_string()))
                            .await
                            .is_err()
                        {
                            tracing::error!("Websocket sender has been closed, aborting websocket");
                            break;
                        }

                        if !receiver.is_closed() {
                            let subscription_responses = socket_responses.clone();
                            tokio::task::spawn(async move {
                                tracing::trace!("Spawning subscription responses loop");
                                while let Some(message) = receiver.recv().await {
                                    tracing::trace!("Subscription message received: {}", message);
                                    if let Err(error) = subscription_responses
                                        .send(Message::Text(message.to_string()))
                                        .await
                                    {
                                        tracing::error!(%error, "Error while sending RPC response");
                                    }
                                }
                                tracing::trace!("Subscription channel closed");
                            });
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "Error while processing RPC request");
                    }
                }
            }
            // NOTE: No support for binary formats.
            Message::Binary(_) => {
                tracing::warn!("Binary JSON RPC messages are not supported");
            }
            Message::Pong(_) => {}
            Message::Ping(ping) => {
                if socket_responses.send(Message::Pong(ping)).await.is_err() {
                    tracing::error!("Websocket sender has been closed, aborting websocket");
                    break;
                }
            }
            Message::Close(_) => {
                break;
            }
        }
    }
    tracing::trace!("WebSocket read handler finished");
}

async fn handle_socket_write(
    mut socket_requests: tokio::sync::mpsc::Receiver<Message>,
    mut socket_responses: futures_util::stream::SplitSink<WebSocket, Message>,
) {
    while let Some(response) = socket_requests.recv().await {
        if let Err(error) = socket_responses.send(response).await {
            tracing::error!(%error, "Error while sending RPC response");
        }
        tracing::trace!("Message sent to websocket");
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
    use jsonrpsee::core::JsonRawValue;
    use jsonrpsee::ws_client::WsClientBuilder;

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

    // Returns shutdown sender
    async fn build_and_start_test_server() -> (SocketAddr, watch::Sender<()>) {
        let methods = build_test_json_rpc();
        let axum_router = build_test_axum_router();
        let (shutdown_sender, mut shutdown_receiver) = watch::channel(());
        shutdown_receiver.mark_unchanged();
        let (_join_handle, addr) = start_http_server(
            &SocketAddr::from(([127, 0, 0, 1], 0)),
            axum_router,
            methods,
            shutdown_receiver,
            CorsConfiguration::Restrictive,
        )
        .await
        .unwrap();

        (addr, shutdown_sender)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_request_response() -> anyhow::Result<()> {
        let (addr, shutdown_sender) = build_and_start_test_server().await;

        let ws_client = WsClientBuilder::default()
            .build(&format!("ws://{addr}/rpc"))
            .await?;

        for _ in 0..10 {
            let response = ws_client
                .request::<String, [u8; 0]>(RPC_READ_METHOD, [])
                .await?;

            assert_eq!(response, "hi");
        }
        shutdown_sender.send(())?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_subscription() -> anyhow::Result<()> {
        let (addr, shutdown_sender) = build_and_start_test_server().await;

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

        shutdown_sender.send(())?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ws_duplex() -> anyhow::Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(3), test_ws_duplex_inner()).await?
    }

    async fn test_ws_duplex_inner() -> anyhow::Result<()> {
        let (addr, shutdown_sender) = build_and_start_test_server().await;

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

        shutdown_sender.send(())?;

        Ok(())
    }
}

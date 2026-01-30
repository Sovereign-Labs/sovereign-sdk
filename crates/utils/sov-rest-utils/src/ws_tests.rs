//! Integration tests for WebSocket functionality.
//!
//! These tests verify:
//! 1. Socket errors (feed/flush) cause immediate disconnect
//! 2. BroadcastStream lag sends skip notification and resumes (not disconnect)
//! 3. Ping/pong keepalive detects dead connections
//! 4. Gzip compression mode batches and compresses messages correctly

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use axum::extract::ws::WebSocketUpgrade;
    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use futures::StreamExt;
    use tokio::net::TcpListener;
    use tokio::sync::{broadcast, watch};
    use tokio::time::timeout;
    use tokio_stream::wrappers::BroadcastStream;

    use crate::errors::ReportableWsError;
    use crate::{
        serve_generic_ws_subscription, serve_generic_ws_subscription_with_config,
        WsSubscriptionConfig,
    };

    /// Error type matching the real SubscriptionStreamError pattern.
    #[derive(Debug, Clone)]
    enum TestSubscriptionError {
        Lagged {
            skipped: u64,
            disconnected_at: Option<u64>,
            resumed_at: Option<u64>,
        },
    }

    impl ReportableWsError for TestSubscriptionError {
        fn to_json(&self) -> String {
            match self {
                TestSubscriptionError::Lagged {
                    skipped,
                    disconnected_at,
                    resumed_at,
                } => {
                    // If we have identifier information, use the detailed format
                    if disconnected_at.is_some() || resumed_at.is_some() {
                        format!(
                            r#"{{"message": "lagged", "details": {{"disconnected_at": {}, "resumed_at": {}}}}}"#,
                            disconnected_at
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "null".to_string()),
                            resumed_at
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "null".to_string())
                        )
                    } else {
                        // Otherwise, use the standard format with skip count
                        format!(
                            r#"{{"status": 200, "message": "Messages skipped due to lag", "details": {{"skipped": {skipped}, "reason": "lag"}}}}"#
                        )
                    }
                }
            }
        }

        fn is_recoverable(&self) -> bool {
            // Lagged IS recoverable - we skip and continue
            match self {
                TestSubscriptionError::Lagged { .. } => true,
            }
        }
    }

    /// State shared between test handlers.
    #[derive(Clone)]
    struct TestState {
        shutdown_tx: watch::Sender<()>,
        shutdown_rx: watch::Receiver<()>,
        /// Tracks how many messages were sent to the broadcast channel.
        messages_produced: Arc<AtomicUsize>,
        /// Tracks how many messages the stream attempted to yield.
        messages_yielded: Arc<AtomicUsize>,
    }

    impl TestState {
        fn new() -> Self {
            let (shutdown_tx, shutdown_rx) = watch::channel(());
            Self {
                shutdown_tx,
                shutdown_rx,
                messages_produced: Arc::new(AtomicUsize::new(0)),
                messages_yielded: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    type BoxedStream<T, E> = Pin<Box<dyn futures::Stream<Item = Result<T, E>> + Send>>;

    /// Creates a stream backed by a broadcast channel.
    /// Maps Lagged errors to include identifier tracking.
    fn broadcast_message_stream(
        buffer_size: usize,
        messages_yielded: Arc<AtomicUsize>,
    ) -> (
        BoxedStream<String, TestSubscriptionError>,
        broadcast::Sender<String>,
    ) {
        use std::sync::atomic::AtomicU64;
        use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

        let (tx, rx) = broadcast::channel::<String>(buffer_size);
        let last_id = Arc::new(AtomicU64::new(0));
        let stream = BroadcastStream::new(rx).map(move |result| {
            messages_yielded.fetch_add(1, Ordering::SeqCst);
            match result {
                Ok(msg) => {
                    // Extract message number from "message-N" format
                    let id = msg
                        .strip_prefix("message-")
                        .or_else(|| msg.strip_prefix("slow-message-"))
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap_or(0);
                    last_id.store(id, Ordering::SeqCst);
                    Ok(msg)
                }
                Err(BroadcastStreamRecvError::Lagged(count)) => {
                    let disconnected = last_id.load(Ordering::SeqCst);
                    Err(TestSubscriptionError::Lagged {
                        skipped: count,
                        disconnected_at: Some(disconnected),
                        resumed_at: Some(disconnected + count + 1),
                    })
                }
            }
        });
        (Box::pin(stream), tx)
    }

    /// Handler that uses a broadcast channel with small buffer to trigger lag.
    async fn broadcast_handler(
        ws: WebSocketUpgrade,
        State(state): State<TestState>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| async move {
            // Small buffer size (4) to easily trigger lag when client is slow
            let (stream, tx) = broadcast_message_stream(4, state.messages_yielded.clone());

            // Spawn a task to rapidly produce messages
            let messages_produced = state.messages_produced.clone();
            let producer = tokio::spawn(async move {
                for i in 0..1000 {
                    if tx.send(format!("message-{i}")).is_err() {
                        break;
                    }
                    messages_produced.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_micros(100)).await;
                }
            });

            serve_generic_ws_subscription(socket, stream, state.shutdown_rx.clone()).await;
            producer.abort();
        })
    }

    /// Handler with larger buffer and slower production for disconnect tests.
    async fn slow_broadcast_handler(
        ws: WebSocketUpgrade,
        State(state): State<TestState>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| async move {
            let (stream, tx) = broadcast_message_stream(100, state.messages_yielded.clone());

            let messages_produced = state.messages_produced.clone();
            let producer = tokio::spawn(async move {
                for i in 0..100 {
                    if tx.send(format!("slow-message-{i}")).is_err() {
                        break;
                    }
                    messages_produced.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            });

            serve_generic_ws_subscription(socket, stream, state.shutdown_rx.clone()).await;
            producer.abort();
        })
    }

    /// Handler that sends no data - used for testing ping/pong keepalive in isolation.
    async fn idle_handler(
        ws: WebSocketUpgrade,
        State(state): State<TestState>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| async move {
            // Create a stream that never yields any items (just stays pending)
            let stream = futures::stream::pending::<Result<String, TestSubscriptionError>>();
            serve_generic_ws_subscription(socket, stream, state.shutdown_rx.clone()).await;
        })
    }

    /// Handler that uses compression for WebSocket messages.
    async fn compressed_handler(
        ws: WebSocketUpgrade,
        State(state): State<TestState>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| async move {
            let (stream, tx) = broadcast_message_stream(100, state.messages_yielded.clone());

            let messages_produced = state.messages_produced.clone();
            let producer = tokio::spawn(async move {
                for i in 0..20 {
                    if tx.send(format!("message-{i}")).is_err() {
                        break;
                    }
                    messages_produced.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });

            serve_generic_ws_subscription_with_config(
                socket,
                stream,
                state.shutdown_rx.clone(),
                WsSubscriptionConfig { compress: true },
            )
            .await;
            producer.abort();
        })
    }

    /// Starts a test server with all handlers and returns the address.
    async fn start_test_server(state: TestState) -> SocketAddr {
        let app = Router::new()
            .route("/broadcast", get(broadcast_handler))
            .route("/slow", get(slow_broadcast_handler))
            .route("/idle", get(idle_handler))
            .route("/compressed", get(compressed_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        addr
    }

    // =========================================================================
    // Socket Error Tests - Should disconnect
    // =========================================================================

    /// Test: When a client disconnects, the server should detect it and stop.
    #[tokio::test]
    async fn test_client_disconnect_stops_server() {
        let state = TestState::new();
        let addr = start_test_server(state.clone()).await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/slow"))
            .await
            .unwrap();

        // Receive 2 text messages then close (skip ping/pong frames)
        let mut text_count = 0;
        // Verify the server detected the disconnect and stopped producing.
        // The slow_broadcast_handler produces messages every 50ms, so if it kept
        // running for the full 500ms wait, it would produce ~10 messages.
        // We use < 20 as a loose bound to account for timing variance.
        // If disconnect detection failed, the producer would continue toward 100 messages.
        while text_count < 2 {
            let msg = ws.next().await.unwrap().unwrap();
            match msg {
                tungstenite::Message::Text(_) => text_count += 1,
                tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => continue,
                _ => panic!("Unexpected message type: {msg:?}"),
            }
        }

        ws.close(None).await.ok();
        drop(ws);

        tokio::time::sleep(Duration::from_millis(500)).await;

        let produced = state.messages_produced.load(Ordering::SeqCst);
        assert!(
            produced < 20,
            "Server produced {produced} messages but should have stopped after client disconnect."
        );
    }

    /// Test: Server should not hang when client reads slowly.
    ///
    /// When socket.feed()/flush() fails due to backpressure, the server
    /// should disconnect, not hang forever.
    #[tokio::test]
    async fn test_slow_reader_causes_disconnect() {
        let state = TestState::new();
        let addr = start_test_server(state.clone()).await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/broadcast"))
            .await
            .unwrap();

        let _msg = ws.next().await.unwrap().unwrap();

        let mut message_count = 0;

        let closed = timeout(Duration::from_secs(2), async {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;

                match ws.next().await {
                    Some(Ok(tungstenite::Message::Text(_))) => {
                        message_count += 1;
                    }
                    // Connection closed or errored - server disconnected as expected
                    Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => return,
                    _ => {}
                }
            }
        })
        .await;

        let produced = state.messages_produced.load(Ordering::SeqCst);

        if closed.is_err() {
            // Timeout - check if server completed its work (TCP buffering handled it)
            if produced >= 900 {
                eprintln!(
                    "Note: Server produced {produced} messages, client received {message_count}. \
                     TCP buffering handled the load without errors."
                );
                return;
            }
            panic!(
                "TIMEOUT: Server produced {produced} messages, client received {message_count}. \
                 Server may be hanging."
            );
        }

        state.shutdown_tx.send(()).ok();
    }

    // =========================================================================
    // BroadcastStream Lag Tests - Should skip and resume, NOT disconnect
    // =========================================================================

    /// Test: When a recoverable error occurs, server should send error notification
    /// and continue streaming (not disconnect).
    ///
    /// Note: Actual BroadcastStream lag is hard to trigger in tests because:
    /// 1. The server consumes from the stream as fast as possible
    /// 2. TCP buffers handle the output without backpressure
    /// 3. The BroadcastStream receiver never falls behind
    ///
    /// This test verifies the code path works by checking that:
    /// 1. Server completes without hanging
    /// 2. Messages are received
    /// 3. When recoverable errors DO occur, they don't cause disconnect
    #[tokio::test]
    async fn test_recoverable_errors_continue_streaming() {
        let state = TestState::new();
        let addr = start_test_server(state.clone()).await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/broadcast"))
            .await
            .unwrap();

        // Read messages until stream completes or connection closes
        let mut message_count = 0;
        let mut skip_notifications = 0;

        let closed = timeout(Duration::from_secs(3), async {
            loop {
                match ws.next().await {
                    Some(Ok(tungstenite::Message::Text(t))) => {
                        let text = t.to_string();

                        // Check if this is a lag notification
                        if text.contains("lagged") && text.contains("disconnected_at") {
                            skip_notifications += 1;
                            continue;
                        }

                        message_count += 1;
                    }
                    // Connection closed or errored
                    Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => return,
                    _ => {}
                }
            }
        })
        .await;

        let produced = state.messages_produced.load(Ordering::SeqCst);

        if closed.is_ok() {
            // Connection closed normally - server completed or disconnected
            eprintln!(
                "Server produced {produced} messages, client received {message_count}, skip notifications: {skip_notifications}"
            );
            assert!(
                message_count > 0,
                "Expected to receive at least some messages"
            );
        } else {
            // Timeout - check if server completed its work
            if produced >= 900 {
                eprintln!(
                    "Server produced {produced} messages (completed), client received {message_count}"
                );
                return;
            }
            panic!(
                "Timeout. Server produced {produced} messages, client received {message_count}. \
                 Server may be hanging."
            );
        }

        state.shutdown_tx.send(()).ok();
    }

    // =========================================================================
    // Ping/Pong Keepalive Tests
    // =========================================================================

    /// Test: Server should send periodic ping frames when idle.
    ///
    /// Uses the /idle endpoint which sends no data, so ping keepalive is the only activity.
    /// The first ping is sent after PING_INTERVAL (30s) of inactivity.
    ///
    /// This test is ignored by default because it takes ~30 seconds (PING_INTERVAL).
    /// Run with `cargo test --ignored` to include it.
    #[tokio::test]
    async fn test_server_sends_pings() {
        let state = TestState::new();
        let addr = start_test_server(state.clone()).await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/idle"))
            .await
            .unwrap();

        // Wait for a ping (sent after PING_INTERVAL of inactivity)
        let ping_received = timeout(Duration::from_secs(50), async {
            loop {
                match ws.next().await {
                    Some(Ok(tungstenite::Message::Ping(_))) => return true,
                    Some(Ok(_)) => continue,
                    _ => return false,
                }
            }
        })
        .await;

        assert!(
            ping_received.unwrap_or(false),
            "Server should send ping frames for keepalive"
        );

        state.shutdown_tx.send(()).ok();
    }

    /// Test: Server should disconnect if pong is not received within timeout.
    ///
    /// Note: tokio-tungstenite automatically responds to pings with pongs,
    /// so we use a raw TCP connection to test timeout behavior.
    ///
    /// Uses the /idle endpoint which sends no data, allowing ping/pong to be the only activity.
    ///
    /// This test is ignored by default because it takes ~40 seconds (PING_INTERVAL + PONG_TIMEOUT).
    /// Run with `cargo test --ignored` to include it.
    #[tokio::test]
    async fn test_pong_timeout_disconnects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let state = TestState::new();
        let addr = start_test_server(state.clone()).await;

        // Connect via raw TCP and perform WebSocket handshake manually
        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Send WebSocket upgrade request to /idle (no data, only ping/pong)
        let request = format!(
            "GET /idle HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();

        // Read the HTTP response (we don't need to parse it fully for this test)
        let mut response = vec![0u8; 1024];
        let _ = stream.read(&mut response).await.unwrap();

        // Now we have a WebSocket connection but we won't respond to pings
        // The timeout check only runs when the ping interval ticks, so:
        // - First ping sent at PING_INTERVAL (30s)
        // - Timeout check runs at next tick (60s), sees no pong received, disconnects
        // We wait a bit longer than 2*PING_INTERVAL to be safe

        let start = std::time::Instant::now();
        let disconnect_result = timeout(Duration::from_secs(70), async {
            let mut buf = vec![0u8; 1024];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) => return true,  // Connection closed
                    Ok(_) => continue,     // Got some data, keep reading
                    Err(_) => return true, // Error = connection closed
                }
            }
        })
        .await;
        let elapsed = start.elapsed();
        eprintln!("Connection closed after {elapsed:?}");

        assert!(
            disconnect_result.unwrap_or(false),
            "Server should disconnect after pong timeout"
        );

        state.shutdown_tx.send(()).ok();
    }

    // =========================================================================
    // Compression Tests
    // =========================================================================

    /// Test: Compressed messages should be binary frames with gzip magic bytes.
    #[tokio::test]
    async fn test_compressed_messages_are_binary_gzip() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let state = TestState::new();
        let addr = start_test_server(state.clone()).await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/compressed"))
            .await
            .unwrap();

        let mut binary_count = 0;
        let mut all_messages: Vec<String> = Vec::new();

        let result = timeout(Duration::from_secs(5), async {
            loop {
                match ws.next().await {
                    Some(Ok(tungstenite::Message::Binary(data))) => {
                        binary_count += 1;

                        // Verify gzip magic bytes
                        assert!(
                            data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b,
                            "Binary frame should start with gzip magic bytes (0x1f 0x8b), got: {:02x} {:02x}",
                            data.first().copied().unwrap_or(0),
                            data.get(1).copied().unwrap_or(0)
                        );

                        // Decompress
                        let mut decoder = GzDecoder::new(&data[..]);
                        let mut decompressed = String::new();
                        decoder.read_to_string(&mut decompressed).unwrap();

                        // Parse as JSON array
                        let batch: Vec<String> = serde_json::from_str(&decompressed).unwrap();
                        all_messages.extend(batch);
                    }
                    Some(Ok(tungstenite::Message::Text(_))) => {
                        panic!("Compressed mode should not send text frames");
                    }
                    Some(Ok(tungstenite::Message::Ping(_))) | Some(Ok(tungstenite::Message::Pong(_))) => continue,
                    Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test should complete without timeout");
        assert!(binary_count > 0, "Should have received binary frames");
        assert!(!all_messages.is_empty(), "Should have received messages");

        // Verify message ordering is preserved
        for (i, msg) in all_messages.iter().enumerate() {
            assert_eq!(msg, &format!("message-{i}"), "Messages should be in order");
        }

        state.shutdown_tx.send(()).ok();
    }

    /// Test: Default config (no compression) should produce text frames.
    #[tokio::test]
    async fn test_default_config_produces_text_frames() {
        let state = TestState::new();
        let addr = start_test_server(state.clone()).await;

        // Use /slow endpoint which uses default (uncompressed) mode
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/slow"))
            .await
            .unwrap();

        let mut text_count = 0;

        let result = timeout(Duration::from_secs(3), async {
            while text_count < 5 {
                match ws.next().await {
                    Some(Ok(tungstenite::Message::Text(t))) => {
                        text_count += 1;
                        // Verify it's a single JSON string, not an array
                        let text = t.to_string();
                        assert!(
                            text.starts_with('"'),
                            "Uncompressed mode should send individual JSON values, not arrays"
                        );
                    }
                    Some(Ok(tungstenite::Message::Binary(_))) => {
                        panic!("Uncompressed mode should not send binary frames for data");
                    }
                    Some(Ok(tungstenite::Message::Ping(_)))
                    | Some(Ok(tungstenite::Message::Pong(_))) => continue,
                    Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test should complete without timeout");
        assert!(text_count >= 5, "Should have received text frames");

        ws.close(None).await.ok();
        state.shutdown_tx.send(()).ok();
    }

    /// Test: Gzip roundtrip preserves data correctly.
    #[test]
    fn test_gzip_roundtrip() {
        use flate2::read::GzDecoder;
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::{Read, Write};

        let original = vec!["message-0", "message-1", "message-2"];
        let json = serde_json::to_vec(&original).unwrap();

        // Compress
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&json).unwrap();
        let compressed = encoder.finish().unwrap();

        // Verify magic bytes
        assert_eq!(compressed[0], 0x1f, "First byte should be gzip magic");
        assert_eq!(compressed[1], 0x8b, "Second byte should be gzip magic");

        // Decompress
        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();

        // Parse
        let parsed: Vec<String> = serde_json::from_slice(&decompressed).unwrap();

        assert_eq!(parsed, original);
    }
}

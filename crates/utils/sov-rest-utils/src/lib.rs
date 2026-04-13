// NOTE: this crate can be used as a standalone crate, but most rollup
// developers will interface with it through `sov_modules_api::rest`. So, keep
// that in mind when writing docs.

//! Utilities for building opinionated REST(ful) APIs with [`axum`].
//!
//! # Design choices
//! - Response and request formats are *occasionally* inspired by
//!   [JSON:API](https://jsonapi.org/format/). This crate does *NOT* aim to be
//!   JSON:API compliant. More specifically, we completely disregard any parts
//!   of the spec that we find unnecessary or problematic for most use cases
//!   (e.g. "link objects" and "relationships", which only make sense when
//!   designing truly  HATEOAS-driven APIs).
//! - Query string parameters follow the bracket notation `foo[bar]` that was
//!   popularized by [`qs`](https://github.com/ljharb/qs).
//! - Pagination is cursor-based.
//!
//! # Missing features
//! - Multi-column sorting (see <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/449>).

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod axum_extractors;
mod filter;
mod get_ip;
mod pagination;
mod sorting;

pub mod errors;

#[doc(hidden)]
#[cfg(test)]
pub mod test_utils;

#[cfg(test)]
mod ws_tests;
use axum::body::Body;
use axum::extract::ws::WebSocket;
use axum::extract::Request;
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Error, Json, Router};
pub use axum_extractors::{Path, Query};
pub use filter::{Filter, FilterError, FilterQuery};
use futures::{SinkExt, StreamExt};
pub use get_ip::*;
pub use pagination::{PageSelection, PaginatedResponse, Pagination};
use serde::Serialize;
pub use sorting::{Sorting, SortingOrder};
use std::fmt::Debug;
use tower_http::cors::CorsLayer;
use tower_http::propagate_header::PropagateHeaderLayer;
use tower_http::trace::TraceLayer;
use tower_request_id::{RequestId, RequestIdLayer};
use tracing::{error, error_span, trace, warn};

use crate::errors::ReportableWsError;

/// Standard result type for API endpoints.
pub type ApiResult<T> = Result<axum::Json<T>, Response>;

impl IntoResponse for ErrorObject {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}

/// A JSON object (mind you, not a *value*, but an
/// [*object*](https://www.json.org/json-en.html)).
pub type JsonObject = serde_json::Map<String, serde_json::Value>;

/// Inspired from <https://jsonapi.org/format/#error-objects>.
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ErrorObject {
    /// The HTTP status that best describes the error.
    #[serde(with = "serde_status_code")]
    pub status: StatusCode,
    /// A short, human-readable description of the error.
    pub message: String,
    /// Structured details about the error, if available.
    pub details: JsonObject,
}

mod serde_status_code {
    use axum::http::StatusCode;
    use serde::Deserialize;

    pub fn serialize<S>(status_code: &StatusCode, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u16(status_code.as_u16())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<StatusCode, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let code = u16::deserialize(deserializer)?;
        StatusCode::from_u16(code).map_err(serde::de::Error::custom)
    }
}

/// Exactly like [`serde_json::Value`], but returns a JSON object instead of a
/// JSON value.
#[macro_export]
macro_rules! json_obj {
    ($($json:tt)+) => {
        $crate::to_json_object(::serde_json::json!($($json)+))
    };
}

/// Calls [`serde_json::to_value`] on the given value but panics if the
/// resulting value is not a JSON object.
pub fn to_json_object<T: Serialize>(value: T) -> JsonObject {
    let value = serde_json::to_value(value).unwrap();
    match value {
        serde_json::Value::Object(obj) => obj,
        _ => panic!("Expected serialization to produce a JSON object; got {value:?}"),
    }
}

/// Customizes the given [`Router`] with a set of preconfigured "layers" that
/// are a good starting point for building production-ready JSON APIs.
pub fn preconfigured_router_layers<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // Tracing span with unique ID per request:
    // <https://github.com/imbolc/tower-request-id/blob/main/examples/logging.rs>
    let trace_layer = TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
        // We get the request id from the extensions
        let request_id = request
            .extensions()
            .get::<RequestId>()
            .map(ToString::to_string)
            .unwrap_or_else(|| "unknown".into());
        // And then we put it along with other information into the `request` span
        error_span!(
            "request",
            id = %request_id,
            method = %request.method(),
            uri = %request.uri(),
        )
    });
    router
        .layer(trace_layer)
        // This layer creates a new id for each request and puts it into the request extensions.
        // Note that it should be added after the Trace layer. (Filippo: why? I
        // don't know, I copy-pasted this.)
        .layer(RequestIdLayer)
        .layer(
            tower::ServiceBuilder::new()
                // Tracing.
                .layer(TraceLayer::new_for_http())
                // Propagate `X-Request-Id`s from requests to responses.
                .layer(PropagateHeaderLayer::new(HeaderName::from_static(
                    "x-request-id",
                ))),
        )
}

/// A pre-configured [`CorsLayer`] with permissive configurations.
///
/// Note that  allowing CORS is necessary for Metamask Snap.
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(tower_http::cors::Any) // Allow all origins
        .allow_methods(tower_http::cors::Any) // Allow all methods
        .allow_headers(tower_http::cors::Any) // Allow all headers
}

/// Optional CORS layer.
pub fn cors_layer_opt(
    enable: bool,
) -> tower::util::Either<CorsLayer, tower::layer::util::Identity> {
    tower::util::option_layer(if enable { Some(cors_layer()) } else { None })
}

const MAX_BATCH_SIZE: usize = 128;

/// Configuration for WebSocket subscription behavior.
#[derive(Debug, Clone, Copy, Default)]
pub struct WsSubscriptionConfig {
    /// When true, messages are batched into arrays and gzip-compressed before sending.
    pub compress: bool,
}

/// Interval between ping frames sent to the client for keepalive.
const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Maximum time to wait for a pong response before considering the connection dead.
const PONG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A utility function for serving some data inside a [`futures::Stream`] over a
/// WebSocket connection.
///
/// This function handles:
/// - Sending data from the subscription stream to the client
/// - Periodic ping/pong keepalive to detect dead connections
/// - Graceful shutdown on server shutdown signal
/// - Proper handling of client disconnection and half-closed connections
pub async fn serve_generic_ws_subscription<S, M, E>(
    socket: WebSocket,
    subscription: S,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
) where
    S: futures::Stream<Item = Result<M, E>> + Unpin,
    E: ReportableWsError,
    M: Clone + serde::Serialize + Send + Sync + 'static,
{
    serve_generic_ws_subscription_with_config(
        socket,
        subscription,
        shutdown_receiver,
        WsSubscriptionConfig::default(),
    )
    .await
}

/// A utility function for serving some data inside a [`futures::Stream`] over a
/// WebSocket connection, with configurable behavior.
///
/// This function handles:
/// - Sending data from the subscription stream to the client
/// - Periodic ping/pong keepalive to detect dead connections
/// - Graceful shutdown on server shutdown signal
/// - Proper handling of client disconnection and half-closed connections
/// - Optional gzip compression of batched messages (when `config.compress` is true)
///
/// When compression is enabled:
/// - Data messages are batched into JSON arrays, gzip-compressed, and sent as binary frames
/// - Error messages are also gzip-compressed and sent as binary frames
/// - Clients can uniformly decompress all binary frames (gzip magic bytes: `0x1f 0x8b`)
pub async fn serve_generic_ws_subscription_with_config<S, M, E>(
    mut socket: WebSocket,
    subscription: S,
    mut shutdown_receiver: tokio::sync::watch::Receiver<()>,
    config: WsSubscriptionConfig,
) where
    S: futures::Stream<Item = Result<M, E>> + Unpin,
    E: ReportableWsError,
    M: Clone + serde::Serialize + Send + Sync + 'static,
{
    use axum::extract::ws::Message;
    use std::time::Instant;

    // Use ready_chunks to automatically batch items that are immediately available
    let mut chunked_subscription = subscription.ready_chunks(MAX_BATCH_SIZE);

    // Ping/pong state for keepalive
    // Use interval_at to delay the first ping until after a full interval of inactivity
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut awaiting_pong: Option<[u8; 8]> = None;
    let mut ping_sent_time = Instant::now();
    let mut ping_counter: u64 = 0;

    'outer: loop {
        tokio::select! {
            // Biased ensures we check recv first to handle Close frames promptly
            biased;

            msg = socket.recv() => {
                match msg {
                    Some(Err(error)) => {
                        warn!(?error, "WebSocket error");
                        break;
                    },
                    None => {
                        // The client disconnected (received Close frame or connection closed).
                        trace!("WebSocket connection closed by client");
                        break;
                    },
                    Some(Ok(Message::Pong(data))) => {
                        // Client responded to our ping - verify it matches what we sent
                        if awaiting_pong.is_some_and(|expected| *data == expected) {
                            awaiting_pong = None;
                            trace!("Received valid pong from client");
                        } else {
                            trace!("Received pong with unexpected data; ignoring");
                        }
                    },
                    Some(Ok(Message::Ping(data))) => {
                        // Respond to client pings (though clients typically don't send pings)
                        if let Err(err) = socket.send(Message::Pong(data)).await {
                            warn!(?err, "Failed to send pong - disconnecting client");
                            break;
                        }
                    },
                    Some(Ok(Message::Close(_))) => {
                        // Client initiated close - acknowledge and exit
                        trace!("Received close frame from client");
                        break;
                    },
                    Some(Ok(_)) => {
                        // Client sent an unexpected message - notify them it was ignored
                        let error = ErrorObject {
                            status: StatusCode::BAD_REQUEST,
                            message: "This subscription does not accept incoming messages".to_string(),
                            details: JsonObject::new(),
                        };
                        trace!("Incoming WebSocket message but none was expected; notifying client");
                        if config.compress {
                            if let Err(e) = feed_compressed_bytes(&mut socket, serde_json::to_string(&error).expect("Failed to serialize error as JSON. This is a bug, please report it").as_bytes()).await {
                                warn!(?e, "Failed to send error response - disconnecting client");
                                break;
                            }
                        } else if let Err(e) = send_json(&mut socket, &error).await {
                            warn!(?e, "Failed to send error response - disconnecting client");
                            break;
                        }
                    },
                }
            },
            chunk_opt = chunked_subscription.next() => {
                match chunk_opt {
                    Some(chunk) => {
                        if config.compress {
                            // Compressed mode: batch successful items, compress, send as binary
                            let mut batch: Vec<M> = Vec::with_capacity(chunk.len());
                            for item in chunk {
                                match item {
                                    Ok(data) => {
                                        batch.push(data);
                                    }
                                    Err(err) => {
                                        // Flush any accumulated batch before sending error
                                        if !batch.is_empty() {
                                            if send_compressed_batch(&mut socket, &batch).await.is_err() {
                                                break 'outer;
                                            }
                                            batch.clear();
                                        }

                                        // Send compressed error
                                        if feed_compressed_bytes(&mut socket, err.to_json().as_bytes()).await.is_err() {
                                            break 'outer;
                                        }

                                        if !err.is_recoverable() {
                                            // Note that breaking out of the loop will also flush the socket, so we don't need to do it here.
                                            break 'outer;
                                        }
                                    }
                                }
                            }

                            // Send remaining batch
                            if !batch.is_empty() && send_compressed_batch(&mut socket, &batch).await.is_err() {
                                break 'outer;
                            }
                        } else {
                            // Uncompressed mode: send individual text messages (original behavior)
                            for item in chunk {
                                match item {
                                    Ok(data) => {
                                        let serialized = match serde_json::to_string(&data) {
                                            Ok(serialized) => serialized,
                                            Err(err) => {
                                                error!(?err, "Failed to serialize data for WebSocket; this is a bug, please report it");
                                                break 'outer;
                                            }
                                        };
                                        if let Err(err) = socket.feed(serialized.into()).await {
                                            warn!(?err, "WebSocket send error - disconnecting client");
                                            break 'outer;
                                        }
                                    }
                                    Err(err) => {
                                        // Send error notification to the client
                                        if let Err(send_err) = socket.send(err.to_json().into()).await {
                                            warn!(err=?send_err, "WebSocket send error - disconnecting client");
                                            break 'outer;
                                        }
                                        // For recoverable errors (e.g., lag), continue streaming
                                        // For non-recoverable errors, disconnect
                                        if !err.is_recoverable() {
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                        }
                        if let Err(err) = socket.flush().await {
                            warn!(?err, "WebSocket flush error - disconnecting client");
                            break 'outer;
                        }
                        // Successfully sent data proves connection is alive; reset ping timer
                        ping_interval.reset();
                    },
                    None => {
                        // No more data to send.
                        break;
                    },
                }
            },
            _ = ping_interval.tick() => {
                // Check if we're still waiting for a pong from a previous ping
                if awaiting_pong.is_some() {
                    let elapsed = ping_sent_time.elapsed();
                    if elapsed > PONG_TIMEOUT {
                        warn!("No pong received within timeout ({:?}) - disconnecting client", PONG_TIMEOUT);
                        break;
                    }
                }

                // Send a ping to check if the client is still alive
                ping_counter = ping_counter.wrapping_add(1);
                let ping_data = ping_counter.to_le_bytes();
                if let Err(err) = socket.send(Message::Ping(ping_data.to_vec().into())).await {
                    warn!(?err, "Failed to send ping - disconnecting client");
                    break;
                }
                ping_sent_time = Instant::now();
                awaiting_pong = Some(ping_data);
                trace!("Sent ping to client");
            },
            _ = shutdown_receiver.changed() => break,
        }
    }

    tracing::trace!("Closing websocket subscription");
    socket.send(Message::Close(None)).await.ok();
}

/// Compresses bytes with gzip.
fn compress_bytes(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    encoder.write_all(b"\n")?; // Add a newline for easy use with bash pipes.
    encoder.finish()
}

/// Serializes the value as JSON and compresses it with gzip.
fn compress_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, std::io::Error> {
    let json = serde_json::to_vec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    compress_bytes(&json)
}

/// Compresses and sends a batch of items as a binary WebSocket message.
/// Returns Err(()) if the send fails or compression fails.
async fn send_compressed_batch<T: Serialize>(
    socket: &mut WebSocket,
    batch: &[T],
) -> Result<(), ()> {
    use axum::extract::ws::Message;

    match compress_json(batch) {
        Ok(compressed) => {
            if let Err(err) = socket.feed(Message::Binary(compressed.into())).await {
                warn!(?err, "WebSocket send error - disconnecting client");
                return Err(());
            }
            Ok(())
        }
        Err(err) => {
            error!(
                ?err,
                "Failed to serialize/compress data for WebSocket; this is a bug, please report it"
            );
            Err(())
        }
    }
}

/// Compresses and sends raw bytes as a binary WebSocket message.
/// Returns Err(()) if the send fails or compression fails.
async fn feed_compressed_bytes(socket: &mut WebSocket, data: &[u8]) -> Result<(), anyhow::Error> {
    use axum::extract::ws::Message;

    match compress_bytes(data) {
        Ok(compressed) => {
            if let Err(err) = socket.feed(Message::Binary(compressed.into())).await {
                return Err(err.into());
            }
            Ok(())
        }
        Err(err) => {
            error!(
                ?err,
                "Failed to compress data for WebSocket; this is a bug, please report it"
            );
            Err(err.into())
        }
    }
}

/// A message that can be received via websocket.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct WsMessage<Contents> {
    /// The message id.
    pub id: String,
    /// The contents of the message.
    pub contents: Contents,
}

/// Sends a JSON message over a WebSocket, panicing on serialization failure
pub async fn send_json<Json: Serialize>(socket: &mut WebSocket, json: Json) -> Result<(), Error> {
    let serialized = match serde_json::to_string(&json) {
        Ok(serialized) => serialized,
        Err(err) => {
            error!(
                ?err,
                "Failed to serialize data for WebSocket; this is a bug, please report it"
            );
            panic!("Failed to serialize data for WebSocket; this is a bug, please report it");
        }
    };
    socket.send(serialized.into()).await
}

#[derive(Debug, Clone, Copy)]
/// An error that indicates that the client should be disconnected and the connection should be closed.
pub struct UnrecoverableWsError;

/// Sends a bad request error to the client and returns an UnrecoverableWsError if the message cannot be sent.
pub async fn handle_bad_ws_request(
    socket: &mut WebSocket,
    ip_addr: std::net::IpAddr,
    error: impl ToString,
) -> Result<(), UnrecoverableWsError> {
    if let Err(err) = send_json(
        socket,
        &ErrorObject {
            status: StatusCode::BAD_REQUEST,
            message: "Invalid websocket message".to_string(),
            details: json_obj!({
                "error": error.to_string(),
            }),
        },
    )
    .await
    {
        tracing::warn!(?err, ip_addr=%ip_addr, "Error sending ws message to client");
        return Err(UnrecoverableWsError);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_utils::uri_with_query_params;

    // Ideally we'd also test with types other than strings. E.g. integers?
    #[test_strategy::proptest]
    fn any_query_param_can_be_serialized(key: String, value: String) {
        // As long as it doesn't crash, we're good and the test succeeds.
        uri_with_query_params([(key, value)]);
    }
}

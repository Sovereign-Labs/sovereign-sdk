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
mod pagination;
mod sorting;

pub mod errors;

#[doc(hidden)]
#[cfg(test)]
pub mod test_utils;

use std::fmt::Debug;

use axum::body::Body;
use axum::extract::ws::{self, WebSocket};
use axum::extract::Request;
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
pub use axum_extractors::{Path, Query};
pub use filter::{Filter, FilterError, FilterQuery};
use futures::StreamExt;
pub use pagination::{PageSelection, PaginatedResponse, Pagination};
use serde::Serialize;
pub use sorting::{Sorting, SortingOrder};
use tower_http::cors::CorsLayer;
use tower_http::propagate_header::PropagateHeaderLayer;
use tower_http::trace::TraceLayer;
use tower_request_id::{RequestId, RequestIdLayer};
use tracing::{error, error_span, trace, warn};

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

/// A utility function for serving some data inside a [`futures::Stream`] over a
/// WebSocket connection.
pub async fn serve_generic_ws_subscription<S, M>(
    mut socket: WebSocket,
    mut subscription: S,
    mut shutdown_receiver: tokio::sync::watch::Receiver<()>,
) where
    S: futures::Stream<Item = anyhow::Result<M>> + Unpin,
    M: Clone + serde::Serialize + Send + Sync + 'static,
{
    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Err(error)) => {
                        warn!(?error, "WebSocket error");
                        break;
                    },
                    None => {
                        // The client disconnected.
                        break;
                    },
                    Some(Ok(_)) => {
                        // Ignore incoming messages.
                        trace!("Incoming WebSocket message but none was expected; ignoring");
                    },
                }
            },
            data_res = subscription.next() => {
                match data_res {
                    Some(Ok(data)) => {
                        let serialized = match serde_json::to_string(&data) {
                            Ok(serialized) => serialized,
                            Err(err) => {
                                error!(?err, "Failed to serialize data for WebSocket; this is a bug, please report it");
                                break;
                            }
                        };
                        let message = ws::Message::Text(serialized);
                        if let Err(err) = socket.send(message).await {
                            warn!(?err, "WebSocket error while sending data");
                            // Keep the loop going.
                        }
                    },
                    Some(Err(err)) => {
                        warn!(?err, "WebSocket error while receiving data from internal Tokio channel");
                        break;
                    },
                    None => {
                        // No more data to send.
                        break;
                    },
                }
            }
            _ = shutdown_receiver.changed() => break,
        }
    }

    socket.close().await.ok();
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

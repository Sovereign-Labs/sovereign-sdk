mod extractors;
pub mod models;
mod pagination;
mod sorting;

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{OriginalUri, Query, State};
use axum::http::{HeaderName, Request, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use extractors::ValidatedQuery;
pub use pagination::*;
use serde_json::Value as JsonValue;
pub use sorting::*;
use tower_http::compression::CompressionLayer;
use tower_http::propagate_header::PropagateHeaderLayer;
use tower_http::trace::TraceLayer;
use tower_request_id::{RequestId, RequestIdLayer};
use tracing::error_span;

use self::api_utils::{
    gateway_timeout_response_504, not_found_404, pagination_links, Links, ResourceObject,
    ResponseObject, ResponseObjectData,
};
use self::extractors::PathWithErrorHandling;
use crate::utils::HexString;
use crate::AppState;

type AxumState = State<AppState>;

/// The [`axum::Router`] for the v0 API exposed by the block explorer.
pub fn router(app_state: AppState) -> Router {
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

    Router::new()
        // API design inspired from https://github.com/quantstamp/l2-block-explorer-api/tree/main/open-api
        .route("/blocks", get(get_blocks))
        .route("/blocks/:block_hash", get(get_block_by_hash))
        .route("/transactions", get(get_transactions))
        .route("/transactions/:tx_hash", get(get_tx_by_hash))
        .route("/events", get(get_events))
        .route("/batches/:batch_hash", get(get_batch))
        .route("/batches", get(get_batches))
        .route("/indexing-status", get(get_indexing_status))
        .fallback(api_utils::global_404)
        // Tracing span with unique ID per request:
        // <https://github.com/imbolc/tower-request-id/blob/main/examples/logging.rs>
        .layer(trace_layer)
        // This layer creates a new id for each request and puts it into the request extensions.
        // Note that it should be added after the Trace layer.
        .layer(RequestIdLayer)
        .layer(
            tower::ServiceBuilder::new()
                // Tracing.
                .layer(TraceLayer::new_for_http())
                // Compress responses with GZIP.
                .layer(CompressionLayer::new())
                // Propagate `X-Request-Id`s from requests to responses.
                .layer(PropagateHeaderLayer::new(HeaderName::from_static(
                    "x-request-id",
                ))),
        )
        .with_state(app_state.clone())
        .nest("/extensions/bank", router_bank(app_state.clone()))
        .nest("/extensions/accounts", router_accounts(app_state.clone()))
}

fn router_bank(app_state: AppState) -> Router {
    Router::new()
        .route("/", get(api_utils::not_implemented_501))
        .route("/tokens", get(api_utils::not_implemented_501))
        .route("/tokens/:address", get(api_utils::not_implemented_501))
        .with_state(app_state)
}

fn router_accounts(app_state: AppState) -> Router {
    Router::new()
        .route("/", get(api_utils::not_implemented_501))
        .route("/accounts", get(api_utils::not_implemented_501))
        .route("/accounts/:address", get(api_utils::not_implemented_501))
        .route(
            "/accounts/:address/transactions",
            get(api_utils::not_implemented_501),
        )
        .with_state(app_state)
}

async fn get_tx_by_hash(
    State(state): AxumState,
    PathWithErrorHandling(tx_hash): PathWithErrorHandling<HexString>,
) -> impl IntoResponse {
    let tx = match state.db.get_tx_by_hash(&tx_hash).await {
        Ok(Some(tx)) => tx,
        Ok(None) => return not_found_404("Transaction", tx_hash),
        Err(err) => return gateway_timeout_response_504(err),
    };

    let mut links = Links::new(state.base_url.clone());
    links.add("self", format!("/transactions/{}", tx_hash));

    let data = Some(ResponseObjectData::Single(ResourceObject {
        resoure_type: "transaction",
        id: tx_hash.to_string(),
        attributes: tx,
    }));

    (
        StatusCode::OK,
        Json(ResponseObject {
            data,
            errors: vec![],
            links: links.links(),
        }),
    )
}

async fn get_block_by_hash(
    State(state): AxumState,
    PathWithErrorHandling(block_hash): PathWithErrorHandling<HexString>,
    OriginalUri(uri): OriginalUri,
) -> (StatusCode, Json<ResponseObject<JsonValue>>) {
    let block = match state.db.get_block_by_hash(&block_hash).await {
        Ok(Some(block)) => block,
        Ok(None) => return not_found_404("Block", block_hash),
        Err(err) => return gateway_timeout_response_504(err),
    };

    let mut links = Links::new(state.base_url.clone());
    links.add("self", uri.to_string());

    let response_obj = ResponseObject {
        data: Some(
            ResourceObject {
                resoure_type: "block",
                id: block_hash.to_string(),
                attributes: serde_json::to_value(block).unwrap(),
            }
            .into(),
        ),
        links: links.links(),
        errors: vec![],
    };
    (StatusCode::OK, Json(response_obj))
}

async fn get_events(
    State(state): AxumState,
    ValidatedQuery(params): ValidatedQuery<models::EventsQuery>,
    OriginalUri(uri): OriginalUri,
) -> (StatusCode, Json<ResponseObject<models::Event>>) {
    let events = match state.db.get_events(&params).await {
        Ok(events) => events,
        Err(err) => return gateway_timeout_response_504(err),
    };

    let links = pagination_links(&state, &uri, &params.pagination, "FIXME");
    let response_obj = ResponseObject {
        data: Some(ResponseObjectData::Many(
            events
                .into_iter()
                .map(|event| ResourceObject {
                    resoure_type: "event",
                    id: event.id.to_string(),
                    attributes: event,
                })
                .collect(),
        )),
        errors: vec![],
        links,
    };
    (StatusCode::OK, Json(response_obj))
}

async fn get_blocks(
    State(state): AxumState,
    Query(params): Query<models::BlocksQuery>,
    OriginalUri(uri): OriginalUri,
) -> (StatusCode, Json<ResponseObject<JsonValue>>) {
    let blocks = match state.db.get_blocks(&params).await {
        Ok(blocks) => blocks,
        Err(err) => {
            return gateway_timeout_response_504(err);
        }
    };

    let links = pagination_links(&state, &uri, &params.pagination, "FIXME");
    let response_obj = ResponseObject {
        data: Some(ResponseObjectData::Many(
            blocks
                .into_iter()
                .map(|block| ResourceObject {
                    resoure_type: "block",
                    id: block["hash"].as_str().unwrap().to_string(),
                    attributes: block,
                })
                .collect(),
        )),
        errors: vec![],
        links,
    };
    (StatusCode::OK, Json(response_obj))
}

async fn get_transactions(
    State(state): AxumState,
    ValidatedQuery(params): ValidatedQuery<models::TransactionsQuery>,
    OriginalUri(uri): OriginalUri,
) -> (StatusCode, Json<ResponseObject<JsonValue>>) {
    let txs = match state.db.get_transactions(&params).await {
        Ok(txs) => txs,
        Err(err) => {
            return gateway_timeout_response_504(err);
        }
    };

    let links = pagination_links(&state, &uri, &params.pagination, "FIXME");
    let response_obj = ResponseObject {
        data: Some(ResponseObjectData::Many(
            txs.into_iter()
                .map(|tx| ResourceObject {
                    resoure_type: "transaction",
                    id: tx["hash"].as_str().unwrap().to_string(),
                    attributes: tx,
                })
                .collect(),
        )),
        errors: vec![],
        links,
    };
    (StatusCode::OK, Json(response_obj))
}

async fn get_indexing_status(
    State(state): AxumState,
) -> (StatusCode, Json<ResponseObject<JsonValue>>) {
    let chain_head_opt = match state.db.chain_head().await {
        Ok(chain_head_opt) => chain_head_opt,
        Err(err) => {
            return gateway_timeout_response_504(err);
        }
    };

    let response_obj = ResponseObject {
        data: chain_head_opt.map(|attributes| {
            ResponseObjectData::Single(ResourceObject {
                resoure_type: "indexingStatus",
                id: "latest".to_string(),
                attributes,
            })
        }),
        errors: vec![],
        links: HashMap::new(),
    };
    (StatusCode::OK, Json(response_obj))
}

async fn get_batch(
    State(state): AxumState,
    PathWithErrorHandling(batch_hash): PathWithErrorHandling<HexString>,
) -> (StatusCode, Json<ResponseObject<JsonValue>>) {
    let batch_contents = match state.db.get_batch_by_hash(&batch_hash).await {
        Ok(chain_head_opt) => chain_head_opt,
        Err(err) => return gateway_timeout_response_504(err),
    };

    let response_obj = ResponseObject {
        data: batch_contents.map(|batch| {
            ResponseObjectData::Single(ResourceObject {
                resoure_type: "batch",
                id: batch["hash"]
                    .as_str()
                    .expect("Invalid hash value")
                    .to_string(),
                attributes: batch,
            })
        }),
        errors: vec![],
        links: HashMap::new(),
    };
    (StatusCode::OK, Json(response_obj))
}

async fn get_batches(
    State(state): AxumState,
    ValidatedQuery(params): ValidatedQuery<models::BatchesQuery>,
    OriginalUri(uri): OriginalUri,
) -> (StatusCode, Json<ResponseObject<JsonValue>>) {
    let batches = match state.db.get_batches(&params).await {
        Ok(b) => b,
        Err(err) => return gateway_timeout_response_504(err),
    };

    let links = pagination_links(&state, &uri, &params.pagination, "FIXME");
    let response_obj = ResponseObject {
        data: Some(ResponseObjectData::Many(
            batches
                .into_iter()
                .map(|batch| ResourceObject {
                    resoure_type: "batch",
                    id: batch["hash"].as_str().unwrap().to_string(),
                    attributes: batch,
                })
                .collect(),
        )),
        errors: vec![],
        links,
    };
    (StatusCode::OK, Json(response_obj))
}

/// Helpers for {JSON:API}.
/// See: <https://jsonapi.org/>.
mod api_utils {
    use std::collections::HashMap;

    use super::*;

    use axum::Json;
    use tracing::error;

    pub async fn not_implemented_501() -> (StatusCode, Json<ResponseObject<()>>) {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ResponseObject {
                links: HashMap::new(),
                data: None,
                errors: vec![ErrorObject {
                    status: StatusCode::NOT_IMPLEMENTED.as_u16() as _,
                    title: "Not implemented yet".to_string(),
                    details: None,
                }],
            }),
        )
    }

    pub async fn global_404() -> (StatusCode, Json<ResponseObject<()>>) {
        (
            StatusCode::NOT_FOUND,
            Json(ResponseObject {
                data: None,
                errors: vec![ErrorObject {
                    status: StatusCode::NOT_FOUND.as_u16() as _,
                    title: "Invalid URI".to_string(),
                    details: None,
                }],
                links: HashMap::new(),
            }),
        )
    }

    pub fn not_found_404<T>(
        resource_name_capitalized: &str,
        resource_id: impl ToString,
    ) -> (StatusCode, Json<ResponseObject<T>>) {
        (
            StatusCode::NOT_FOUND,
            Json(ResponseObject {
                data: None,
                errors: vec![ErrorObject {
                    status: StatusCode::NOT_FOUND.as_u16() as _,
                    title: format!("{} not found", resource_name_capitalized),
                    details: Some(format!(
                        "{} '{}' not found",
                        resource_name_capitalized,
                        resource_id.to_string()
                    )),
                }],
                links: HashMap::new(),
            }),
        )
    }

    pub fn gateway_timeout_response_504<T>(
        err: impl ToString,
    ) -> (StatusCode, Json<ResponseObject<T>>) {
        // We don't include the database error in the response, because it may
        // contain sensitive information. But we log it.
        error!(
            error = err.to_string(),
            "Database error while serving request."
        );
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ResponseObject {
                data: None,
                errors: vec![ErrorObject {
                    status: StatusCode::GATEWAY_TIMEOUT.as_u16() as _,
                    title: "Database unavailable".to_string(),
                    details: Some("An error occurred while accessing the database. Please try again later and contact system administrators if the problem persists.".to_string()),
                }],
                links: HashMap::new(),
            }),
        )
    }

    pub fn pagination_links(
        state: &AppState,
        uri: &Uri,
        pagination: &Pagination<i64>,
        new_cursor_value: &str,
    ) -> HashMap<String, String> {
        let full_url = format!("{}{}", state.base_url, uri);
        let mut links = Links::new(state.base_url.clone());
        links.add("self", uri.to_string());
        links.add_pagination_links(&pagination.links(&full_url, new_cursor_value));
        links.links()
    }

    #[derive(Debug, serde::Serialize)]
    pub struct PaginationLinks {
        pub first: String,
        pub last: String,
        pub next: String,
        pub prev: String,
    }

    #[derive(Debug, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ResponseObject<T> {
        #[serde(skip_serializing_if = "HashMap::is_empty")]
        pub links: HashMap<String, String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub data: Option<ResponseObjectData<T>>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub errors: Vec<ErrorObject>,
    }

    #[derive(Debug, serde::Serialize)]
    #[serde(untagged)]
    pub enum ResponseObjectData<T> {
        Single(ResourceObject<T>),
        Many(Vec<ResourceObject<T>>),
    }

    impl<T> From<ResourceObject<T>> for ResponseObjectData<T> {
        fn from(resource: ResourceObject<T>) -> Self {
            Self::Single(resource)
        }
    }

    #[derive(Debug, serde::Serialize)]
    pub struct ResourceObject<T, Id = String> {
        #[serde(rename = "type")]
        pub resoure_type: &'static str,
        pub id: Id,
        pub attributes: T,
    }

    /// Many other fields are available, but we don't need them for now.
    /// See: <https://jsonapi.org/format/#error-objects>.
    #[derive(Debug, serde::Serialize)]
    pub struct ErrorObject {
        pub status: i32,
        pub title: String,
        pub details: Option<String>,
    }

    pub struct Links {
        base_url: String,
        links: HashMap<String, String>,
    }

    impl Links {
        pub fn new(base_url: String) -> Self {
            Self {
                base_url,
                links: HashMap::new(),
            }
        }

        pub fn add(&mut self, name: impl ToString, path: impl AsRef<str>) {
            let mut url = self.base_url.clone();
            url.push_str(path.as_ref());
            self.links.insert(name.to_string(), url);
        }

        pub fn add_pagination_links(&mut self, pag: &PaginationLinks) {
            self.links.insert("first".to_string(), pag.first.clone());
            self.links.insert("last".to_string(), pag.last.clone());
            self.links.insert("next".to_string(), pag.next.clone());
            self.links.insert("prev".to_string(), pag.prev.clone());
        }

        pub fn links(self) -> HashMap<String, String> {
            self.links
        }
    }
}

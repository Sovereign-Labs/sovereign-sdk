pub mod models;
mod pagination;
mod sorting;

use std::collections::HashMap;

use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
pub use pagination::*;
use serde_json::Value as JsonValue;
pub use sorting::*;

use self::jsonapi_utils::{
    bad_request_response, ErrorObject, Links, ResourceObject, ResponseObject, ResponseObjectData,
};
use crate::utils::HexString;
use crate::AppState;

type AxumState = State<AppState>;

pub fn router(app_state: AppState) -> Router {
    Router::new()
        // API design inspired from https://github.com/quantstamp/l2-block-explorer-api/tree/main/open-api
        .route("/blocks", get(get_blocks))
        .route("/blocks/:block_hash", get(get_block_by_hash))
        .route("/transactions", get(get_transactions))
        .route("/transactions/:tx_hash", get(get_tx_by_hash))
        .route("/events", get(get_events))
        .route("/indexing-status", get(get_indexing_status))
        // Unimplemented
        .route("/batches", get(unimplemented))
        .route("/batches/:batch_hash", get(unimplemented))
        .route("/accounts/:address", get(unimplemented))
        .route("/accounts/:address/transactions", get(unimplemented))
        .with_state(app_state)
}

async fn unimplemented() -> Json<ResponseObject<()>> {
    Json(ResponseObject {
        links: HashMap::new(),
        data: None,
        errors: vec![ErrorObject {
            status: 501,
            title: "Not implemented yet".to_string(),
            details: None,
        }],
    })
}

async fn get_tx_by_hash(
    State(state): AxumState,
    Path(tx_hash): Path<HexString>,
) -> impl IntoResponse {
    let tx_opt = state.db.get_tx_by_hash(&tx_hash).await.unwrap();

    if let Some(tx) = tx_opt {
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
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ResponseObject {
                data: None,
                errors: vec![ErrorObject {
                    status: 404,
                    title: "Not found".to_string(),
                    details: None,
                }],
                links: HashMap::new(),
            }),
        )
    }
}

async fn get_block_by_hash(
    State(state): AxumState,
    Path(block_hash): Path<HexString>,
    OriginalUri(uri): OriginalUri,
) -> Json<ResponseObject<JsonValue>> {
    let blocks = state.db.get_block_by_hash(&block_hash).await.unwrap();
    let mut links = Links::new(state.base_url.clone());
    links.add("self", uri.to_string());

    Json(ResponseObject {
        data: Some(
            ResourceObject {
                resoure_type: "block",
                id: block_hash.to_string(),
                attributes: serde_json::to_value(blocks).unwrap(),
            }
            .into(),
        ),
        links: links.links(),
        errors: vec![],
    })
}

async fn get_events(
    State(state): AxumState,
    params: Query<models::EventsQuery>,
) -> Json<ResponseObject<models::Event>> {
    if let Err(err) = params.0.validate() {
        return bad_request_response(err);
    }

    let events = state.db.get_events(&params).await.unwrap();
    Json(ResponseObject {
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
        links: HashMap::new(),
    })
}

async fn get_blocks(
    State(state): AxumState,
    params: Query<models::BlocksQuery>,
) -> Json<ResponseObject<JsonValue>> {
    if let Err(err) = params.validate() {
        return bad_request_response(err);
    }

    let blocks = state.db.get_blocks(&params.0).await.unwrap();

    Json(ResponseObject {
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
        links: HashMap::new(),
    })
}

async fn get_transactions(
    State(state): AxumState,
    params: Query<models::TransactionsQuery>,
) -> Json<ResponseObject<JsonValue>> {
    if let Err(err) = params.validate() {
        return bad_request_response(err);
    }

    let txs = state.db.get_transactions(&params.0).await.unwrap();

    Json(ResponseObject {
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
        links: HashMap::new(),
    })
}

async fn get_indexing_status(State(state): AxumState) -> Json<ResponseObject<JsonValue>> {
    let chain_head_opt = state.db.chain_head().await.unwrap();

    Json(ResponseObject {
        data: Some(ResponseObjectData::Single(ResourceObject {
            resoure_type: "indexingStatus",
            id: "latest".to_string(),
            attributes: chain_head_opt.unwrap_or_default(),
        })),
        errors: vec![],
        links: HashMap::new(),
    })
}

/// Helpers for {JSON:API}.
/// See: <https://jsonapi.org/>.
mod jsonapi_utils {
    use std::collections::HashMap;

    use axum::Json;

    pub fn bad_request_response<T>(err: impl ToString) -> Json<ResponseObject<T>> {
        Json(ResponseObject {
            data: None,
            errors: vec![ErrorObject {
                status: 400,
                title: "Bad request".to_string(),
                details: Some(err.to_string()),
            }],
            links: HashMap::new(),
        })
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

        pub fn links(self) -> HashMap<String, String> {
            self.links
        }
    }
}

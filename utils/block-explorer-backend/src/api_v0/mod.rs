mod jsonapi;

use std::collections::HashMap;

use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value as JsonValue;

use self::jsonapi::{ErrorObject, Links, ResourceObject, ResponseObject, ResponseObjectData};
use crate::{models as m, AppState};

type AxumState = State<AppState>;

pub fn router(app_state: AppState) -> Router {
    Router::new()
        // API design inspired from https://github.com/quantstamp/l2-block-explorer-api/tree/main/open-api
        .route("/blocks", get(get_blocks))
        .route("/blocks/:block_hash", get(get_block_by_hash))
        .route("/transactions", get(get_transactions))
        .route("/transactions/:tx_hash", get(get_tx_by_hash))
        .route("/events", get(get_events))
        // Unimplemented
        .route("/batches", get(unimplemented))
        .route("/batches/:batch_id", get(unimplemented))
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
    Path(tx_hash): Path<m::HexString>,
) -> impl IntoResponse {
    let tx_opt = state.db.get_tx_by_hash(&tx_hash).await.unwrap();

    if let Some(tx) = tx_opt {
        let mut links = Links::new(state.config.clone());
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
    Path(block_hash): Path<m::HexString>,
    OriginalUri(uri): OriginalUri,
) -> Json<ResponseObject<JsonValue>> {
    let blocks = state.db.get_block_by_hash(&block_hash).await.unwrap();
    let mut links = Links::new(state.config.clone());
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
    params: Query<m::EventsQuery>,
) -> Json<ResponseObject<m::Event>> {
    if let Err(err) = params.0.validate() {
        return Json(ResponseObject {
            data: None,
            errors: vec![ErrorObject {
                status: 400,
                title: "Bad request".to_string(),
                details: Some(err.to_string()),
            }],
            links: HashMap::new(),
        });
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

#[derive(Debug, serde::Serialize)]
struct GetBlocksData {
    blocks: Vec<JsonValue>,
}

async fn get_blocks(
    State(state): AxumState,
    params: Query<m::BlocksQuery>,
) -> Json<ResponseObject<JsonValue>> {
    if let Err(err) = params.validate() {
        return Json(ResponseObject {
            data: None,
            errors: vec![ErrorObject {
                status: 400,
                title: "Bad request".to_string(),
                details: Some(err.to_string()),
            }],
            links: HashMap::new(),
        });
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
    params: Query<m::TransactionsQuery>,
) -> Json<ResponseObject<JsonValue>> {
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

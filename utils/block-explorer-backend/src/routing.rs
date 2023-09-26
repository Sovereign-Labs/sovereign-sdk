use crate::{models as m, AppState};
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value as JsonValue};

type AxumState = State<AppState>;

#[derive(Debug, serde::Serialize)]
struct Response<T> {
    pub data: T,
    pub errors: Vec<ResponseError>,
}

#[derive(Debug, serde::Serialize)]
struct ResponseError {
    pub message: String,
}

pub fn api_v0_router(app_state: AppState) -> Router {
    Router::new()
        // API design inspired from https://github.com/quantstamp/l2-block-explorer-api/tree/main/open-api
        .route("/transactions/:tx_hash", get(get_tx_by_hash))
        .route("/events", get(get_events))
        .route("/transactions", get(unimplemented))
        .route("/blocks/:block_id", get(get_block_by_id))
        .route("/blocks", get(unimplemented))
        .route("/batches/:batch_id", get(unimplemented))
        .route("/batches", get(unimplemented))
        .route("/accounts/:address", get(unimplemented))
        .route("/accounts/:address/transactions", get(unimplemented))
        .with_state(app_state)
}

async fn unimplemented() -> Json<JsonValue> {
    Json(json!({
        "error": "unimplemented",
    }))
}

async fn get_tx_by_hash(
    State(state): AxumState,
    Path(tx_hash): Path<m::HexString>,
) -> Json<Response<JsonValue>> {
    let tx_opt = state.db.get_tx_by_hash(&tx_hash.0).await.unwrap();
    Json(Response {
        data: tx_opt.unwrap_or_default(),
        errors: vec![],
    })
}

async fn get_block_by_id(State(state): AxumState, Path(block_id): Path<i64>) -> Json<JsonValue> {
    let blocks = state.db.get_blocks_by_height(block_id).await.unwrap();
    Json(serde_json::to_value(blocks).unwrap())
}

#[derive(Debug, serde::Serialize)]
struct GetEventsData {
    events: Vec<m::Event>,
}

async fn get_events(
    State(state): AxumState,
    params: Query<m::EventsQuery>,
) -> Json<Response<GetEventsData>> {
    let events = state.db.get_events(&params).await.unwrap();
    Json(Response {
        data: GetEventsData { events },
        errors: vec![],
    })
}

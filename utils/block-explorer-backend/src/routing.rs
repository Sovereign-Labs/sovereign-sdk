use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value as JsonValue};

use crate::{models as m, AppState, Config};

type AxumState = State<AppState>;

#[derive(Debug, serde::Serialize)]
struct Response<T> {
    pub data: T,
    pub links: HashMap<String, String>,
    pub errors: Vec<ResponseError>,
}

#[derive(Debug, serde::Serialize)]
struct ResponseError {
    pub status: i32,
    pub source: JsonValue,
    pub title: String,
    pub detail: String,
}

pub fn api_v0_router(app_state: AppState) -> Router {
    Router::new()
        // API design inspired from https://github.com/quantstamp/l2-block-explorer-api/tree/main/open-api
        .route("/blocks", get(get_blocks))
        .route("/blocks/:block_id", get(get_block_by_id))
        .route("/transactions/:tx_hash", get(get_tx_by_hash))
        .route("/transactions", get(unimplemented))
        .route("/events", get(get_events))
        .route("/batches", get(unimplemented))
        .route("/batches/:batch_id", get(unimplemented))
        .route("/accounts/:address", get(unimplemented))
        .route("/accounts/:address/transactions", get(unimplemented))
        .with_state(app_state)
}

async fn unimplemented() -> Json<JsonValue> {
    Json(json!({
        "error": "unimplemented",
    }))
}

struct Links {
    config: Arc<Config>,
    prefix: String,
    links: HashMap<String, String>,
}

impl Links {
    fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            prefix: "/api/v0".to_string(),
            links: HashMap::new(),
        }
    }

    fn add(&mut self, name: impl ToString, path: impl AsRef<str>) {
        let mut url = self.config.base_url.clone();
        url.push_str(&self.prefix);
        url.push_str(path.as_ref());
        self.links.insert(name.to_string(), url);
    }

    fn links(self) -> HashMap<String, String> {
        self.links
    }
}

async fn get_tx_by_hash(
    State(state): AxumState,
    Path(tx_hash): Path<m::HexString>,
) -> Json<Response<JsonValue>> {
    let mut links = Links::new(state.config.clone());
    links.add("self", format!("/transactions/{}", tx_hash));

    let tx_opt = state.db.get_tx_by_hash(&tx_hash.0).await.unwrap();

    Json(Response {
        data: tx_opt.unwrap_or_default(),
        errors: vec![],
        links: links.links(),
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
) -> Json<Response<GetBlocksData>> {
    let blocks = state.db.get_blocks(&params.0).await.unwrap();

    Json(Response {
        data: GetBlocksData { blocks },
        errors: vec![],
        links: HashMap::new(),
    })
}

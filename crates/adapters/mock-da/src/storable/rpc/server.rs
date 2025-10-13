#![allow(dead_code, missing_docs)]

use super::types::*;
use crate::storable::StorableMockDaService;
use crate::{MockBlock, MockDaSpec};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};

use serde_json::Value;
use sov_rollup_interface::da::{DaSpec, RelevantBlobs};
use sov_rollup_interface::node::da::DaService;
use std::net::SocketAddr;

#[derive(Clone)]
pub(crate) struct AppState {
    da_service: StorableMockDaService,
}

// Handler functions
pub(crate) async fn get_block_at_handler(
    Path(height): Path<u64>,
    State(state): State<AppState>,
) -> Result<Json<BlockResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.da_service.get_block_at(height).await {
        Ok(block) => Ok(Json(BlockResponse { block })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )),
    }
}

pub(crate) async fn get_block_header_at_handler(
    Path(height): Path<u64>,
    State(state): State<AppState>,
) -> Result<Json<BlockHeaderResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.da_service.get_block_header_at(height).await {
        Ok(header) => Ok(Json(BlockHeaderResponse { header })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )),
    }
}

pub(crate) async fn get_last_finalized_block_header_handler(
    State(state): State<AppState>,
) -> Result<Json<BlockHeaderResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.da_service.get_last_finalized_block_header().await {
        Ok(header) => Ok(Json(BlockHeaderResponse { header })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )),
    }
}

pub(crate) async fn get_head_block_header_handler(
    State(state): State<AppState>,
) -> Result<Json<BlockHeaderResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.da_service.get_head_block_header().await {
        Ok(header) => Ok(Json(BlockHeaderResponse { header })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )),
    }
}

pub(crate) async fn extract_relevant_blobs_handler(
    State(state): State<AppState>,
    Json(block): Json<MockBlock>,
) -> Result<Json<RelevantBlobsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let blobs = state.da_service.extract_relevant_blobs(&block);
    Ok(Json(RelevantBlobsResponse { blobs }))
}

pub(crate) async fn get_extraction_proof_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Json<ExtractionProofResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Parse the request payload to extract block and blobs
    let block: MockBlock =
        serde_json::from_value(payload.get("block").unwrap().clone()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid block format: {}", e),
                }),
            )
        })?;

    let blobs: RelevantBlobs<<MockDaSpec as DaSpec>::BlobTransaction> =
        serde_json::from_value(payload.get("blobs").unwrap().clone()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid blobs format: {}", e),
                }),
            )
        })?;

    let proofs = state.da_service.get_extraction_proof(&block, &blobs).await;
    Ok(Json(ExtractionProofResponse { proofs }))
}

pub(crate) async fn send_transaction_handler(
    State(state): State<AppState>,
    Json(request): Json<SubmitTransactionRequest>,
) -> Result<Json<SubmitBlobResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Decode hex blob
    let blob = hex::decode(&request.blob).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid hex blob: {}", e),
            }),
        )
    })?;

    let res = state.da_service.send_transaction_inner(&blob).await;

    match res {
        Ok(receipt) => Ok(Json(SubmitBlobResponse { receipt })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Receiver error: {}", e),
            }),
        )),
    }
}

pub(crate) async fn send_proof_handler(
    State(state): State<AppState>,
    Json(request): Json<SubmitProofRequest>,
) -> Result<Json<SubmitBlobResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Decode hex proof data
    let proof_data = hex::decode(&request.aggregated_proof_data).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid hex proof data: {}", e),
            }),
        )
    })?;

    let res = state.da_service.send_proof_inner(&proof_data).await;

    match res {
        Ok(receipt) => Ok(Json(SubmitBlobResponse { receipt })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Receiver error: {}", e),
            }),
        )),
    }
}

pub(crate) async fn get_proofs_at_handler(
    Path(height): Path<u64>,
    State(state): State<AppState>,
) -> Result<Json<ProofsResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.da_service.get_proofs_at(height).await {
        Ok(proofs) => {
            let hex_proofs = proofs.into_iter().map(hex::encode).collect();
            Ok(Json(ProofsResponse { proofs: hex_proofs }))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )),
    }
}

pub(crate) async fn get_signer_handler(
    State(state): State<AppState>,
) -> Result<Json<SignerResponse>, (StatusCode, Json<ErrorResponse>)> {
    let address = state.da_service.get_signer().await;
    Ok(Json(SignerResponse { address }))
}

pub(crate) fn create_router(da_service: StorableMockDaService) -> Router {
    let state = AppState { da_service };

    Router::new()
        .route("/blocks/:height", get(get_block_at_handler))
        .route("/block-headers/:height", get(get_block_header_at_handler))
        .route(
            "/finalized-block-header",
            get(get_last_finalized_block_header_handler),
        )
        .route("/head-block-header", get(get_head_block_header_handler))
        .route(
            "/extract-relevant-blobs",
            post(extract_relevant_blobs_handler),
        )
        .route("/extraction-proof", post(get_extraction_proof_handler))
        .route("/send-transaction", post(send_transaction_handler))
        .route("/send-proof", post(send_proof_handler))
        .route("/proofs/:height", get(get_proofs_at_handler))
        .route("/signer", get(get_signer_handler))
        .with_state(state)
}

pub async fn start_server(da_service: StorableMockDaService) -> SocketAddr {
    let app = create_router(da_service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    addr
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockProducingConfig, MockAddress};
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use crate::storable::layer::StorableMockDaLayer;

    #[tokio::test]
    async fn test_get_head_block_header() {
        let da_layer = Arc::new(RwLock::new(
            StorableMockDaLayer::new_in_memory(0)
                .await
                .expect("Failed to create DA layer"),
        ));
        let da_service = StorableMockDaService::new(
            MockAddress::new([1; 32]),
            da_layer,
            //BlockProducingConfig::Periodic { block_time_ms: 100 },
            BlockProducingConfig::Manual,
        )
        .await;

        da_service.produce_n_blocks_now(10).await;

        let app = create_router(da_service);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/head-block-header").await;
        println!("{:?}, ", response);
        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_send_transaction() {
        let da_layer = Arc::new(RwLock::new(
            StorableMockDaLayer::new_in_memory(0)
                .await
                .expect("Failed to create DA layer"),
        ));
        let da_service = StorableMockDaService::new(
            MockAddress::new([1; 32]),
            da_layer,
            BlockProducingConfig::Periodic { block_time_ms: 100 },
        )
        .await;

        let app = create_router(da_service);
        let server = TestServer::new(app).unwrap();

        let request = SubmitTransactionRequest {
            blob: hex::encode(b"test blob"),
        };

        let response = server.post("/send-transaction").json(&request).await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }
}

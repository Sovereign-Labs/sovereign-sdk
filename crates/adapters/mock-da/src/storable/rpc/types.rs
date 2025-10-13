#![allow(dead_code, missing_docs)]

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sov_rollup_interface::da::{DaSpec, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{DaService, SubmitBlobReceipt};
use std::net::SocketAddr;

use crate::storable::StorableMockDaService;
use crate::{MockBlock, MockDaSpec, MockHash};

#[derive(Serialize, Deserialize)]
pub(crate) struct BlockResponse {
    pub(crate) block: MockBlock,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct BlockHeaderResponse {
    pub(crate) header: <MockDaSpec as DaSpec>::BlockHeader,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SubmitTransactionRequest {
    pub(crate) blob: String, // hex-encoded
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SubmitProofRequest {
    pub(crate) aggregated_proof_data: String, // hex-encoded
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SubmitBlobResponse {
    pub(crate) receipt: SubmitBlobReceipt<MockHash>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct RelevantBlobsResponse {
    pub(crate) blobs: RelevantBlobs<<MockDaSpec as DaSpec>::BlobTransaction>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ExtractionProofResponse {
    pub(crate) proofs: RelevantProofs<
        <MockDaSpec as DaSpec>::InclusionMultiProof,
        <MockDaSpec as DaSpec>::CompletenessProof,
    >,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ExtractionProofRequest {
    pub(crate) block: MockBlock,
    pub(crate) blobs: RelevantBlobs<<MockDaSpec as DaSpec>::BlobTransaction>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ProofsResponse {
    pub(crate) proofs: Vec<String>, // hex-encoded proofs
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SignerResponse {
    pub(crate) address: <MockDaSpec as DaSpec>::Address,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

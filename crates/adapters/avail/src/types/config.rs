use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(JsonSchema, PartialEq, Clone, Deserialize, Serialize)]
pub struct AvailDAConfig {
    // The HTTP API URL of the Avail node
    pub http_api_url: String,
    // The WebSocket API URL of the Avail node
    pub ws_api_url: String,
    // The app IDs to use when submitting proofs and batches
    pub proof_app_id: u32,
    pub batch_app_id: u32,

    // The signer's key to use when submitting proofs and batches
    pub signer_key: String,

    // Backoff policy configurations
    // Minimal time to wait before reattempting to request to avail rpc.
    pub backoff_min_delay_secs: Option<u64>,
    // Maximal time between reattempting to request to the avail rpc.
    pub backoff_max_delay_secs: Option<u64>,
    // Number of requests attempted on avail rpc before returning an error.
    pub backoff_max_times: Option<usize>,
    // Exponential factor for reattempting failed requests
    pub backoff_factor: Option<f32>,
}

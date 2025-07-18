use schemars::JsonSchema;

#[derive(JsonSchema, PartialEq)]
pub struct AvailDAConfig {
    pub http_api_url: String,
    pub ws_api_url: String,
    pub proof_app_id: u32,
    pub batch_app_id: u32,
    pub signer_key: String,
}

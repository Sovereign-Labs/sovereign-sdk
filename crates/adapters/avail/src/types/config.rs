use schemars::JsonSchema;

#[derive(JsonSchema, PartialEq)]
pub struct AvailDAConfig {
    pub http_api_url: String,
    pub ws_api_url: String,
    pub app_id: u64,
    pub signer_key: String,
}

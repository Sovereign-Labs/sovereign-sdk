use crate::da_service::client::twinkle::types::Network;
use crate::da_service::client::twinkle::AGENT;
use schemars::JsonSchema;

#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct TwinkleConfig {
    /// If not set, will be taken from the environment variable `SOV_TWINKLE_API_KEY`
    pub api_key: Option<String>,
    /// Celestia network: "mocha" or "mainnet"
    pub network: Network,
    /// At which interval pull blob status after it has been submitted.
    #[serde(default = "default_pull_interval_millis")]
    pub pull_interval_millis: u64,
    /// Timeout for individual HTTP requests in seconds
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// Timeout for the entire request including retries in seconds
    // TODO:
    pub total_timeout_secs: u64,
    /// Timeout for establishing HTTP connections in seconds
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    /// Timeout for idle connections in the connection pool in seconds
    #[serde(default = "default_pool_idle_timeout_secs")]
    pub pool_idle_timeout_secs: u64,
}

impl std::fmt::Debug for TwinkleConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_api_key = self.api_key.as_ref().map(|_| "<redacted>");
        f.debug_struct("TwinkleConfig")
            .field("api_key", &redacted_api_key)
            .field("network", &self.network)
            .field("pull_interval_millis", &self.pull_interval_millis)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("total_timeout_secs", &self.total_timeout_secs)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field("pool_idle_timeout_secs", &self.pool_idle_timeout_secs)
            .finish()
    }
}

fn default_pull_interval_millis() -> u64 {
    100
}

fn default_request_timeout_secs() -> u64 {
    60
}

fn default_connect_timeout_secs() -> u64 {
    10
}

fn default_pool_idle_timeout_secs() -> u64 {
    30
}

impl TwinkleConfig {
    pub fn test() -> Self {
        Self {
            api_key: Some("TEMP_SECRET".to_string()),
            network: Network::Mocha,
            pull_interval_millis: default_pull_interval_millis(),
            request_timeout_secs: default_request_timeout_secs(),
            // TODO: Use same as in CelestiaConfig.
            total_timeout_secs: 120,
            connect_timeout_secs: default_connect_timeout_secs(),
            pool_idle_timeout_secs: default_pool_idle_timeout_secs(),
        }
    }
    fn api_key(&self) -> String {
        match &self.api_key {
            None => std::env::var("SOV_TWINKLE_API_KEY").expect(
                "SOV_TWINKLE_API_KEY environment is not set, cannot initialize TwinkleClient",
            ),
            Some(set) => set.clone(),
        }
    }

    pub fn construct_reqwest_client(&self) -> reqwest::Client {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut auth_value =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.api_key()))
                .expect("Failed to set TwinkleClient auth");
        auth_value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static(AGENT),
        );

        reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(self.request_timeout_secs))
            .connect_timeout(std::time::Duration::from_secs(self.connect_timeout_secs))
            .pool_idle_timeout(std::time::Duration::from_secs(self.pool_idle_timeout_secs))
            .build()
            .expect("Reqwest HTTP client config should be valid")
    }
}

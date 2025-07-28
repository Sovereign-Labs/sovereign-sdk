use serde::Deserialize;
use sov_node_client::NodeClient;

use crate::generators::basic::BasicClientConfig;

/// An http client for querying the state needed by the value setter generator
pub struct HttpValueSetterClient {
    client: NodeClient,
    rollup_height: Option<u64>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct LenResponse {
    length: u64,
}

#[derive(Debug, Deserialize)]
struct ValueResponse<T> {
    value: Option<T>,
}

#[derive(Debug, Deserialize)]
struct IdxResponse<T> {
    #[allow(unused)]
    index: u64,
    value: Option<T>,
}

impl From<BasicClientConfig> for HttpValueSetterClient {
    fn from(config: BasicClientConfig) -> Self {
        Self {
            rollup_height: config.rollup_height,
            client: NodeClient::new_unchecked(&config.url),
        }
    }
}

impl HttpValueSetterClient {
    pub async fn get_value(&self) -> Option<u32> {
        let rollup_height_param = if let Some(rollup_height) = self.rollup_height {
            format!("?rollup_height={rollup_height}")
        } else {
            String::new()
        };

        let response = self
            .client
            .query_rest_endpoint::<ValueResponse<u32>>(&format!(
                "/modules/value-setter/state/value{rollup_height_param}"
            ))
            .await
            .unwrap();

        response.value
    }

    pub async fn get_many_values_len(&self) -> Option<u64> {
        let rollup_height_param = if let Some(rollup_height) = self.rollup_height {
            format!("?rollup_height={rollup_height}")
        } else {
            String::new()
        };

        let response = self
            .client
            .query_rest_endpoint::<LenResponse>(&format!(
                "/modules/value-setter/state/many-values{rollup_height_param}"
            ))
            .await
            .unwrap();

        Some(response.length)
    }

    pub async fn get_many_values_item(&self, item: u64) -> Option<u8> {
        let rollup_height_param = if let Some(rollup_height) = self.rollup_height {
            format!("?rollup_height={rollup_height}")
        } else {
            String::new()
        };

        self.client
            .query_rest_endpoint::<IdxResponse<u8>>(&format!(
                "/modules/value-setter/state/many-values/items/{item}{rollup_height_param}"
            ))
            .await
            .unwrap()
            .value
    }
}

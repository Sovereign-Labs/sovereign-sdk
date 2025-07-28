use serde::Deserialize;
use sov_modules_api::Spec;
use sov_node_client::NodeClient;
use sov_test_modules::access_pattern::HooksConfig;

use crate::generators::basic::BasicClientConfig;

/// An http client for querying the state needed by the value setter generator
pub struct HttpStorageAccessClient {
    client: NodeClient,
    rollup_height: Option<u64>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct LenResponse {
    length: u64,
}

#[derive(Debug, Deserialize)]
struct MapResponse<T> {
    #[allow(unused)]
    key: u64,
    value: T,
}

impl From<BasicClientConfig> for HttpStorageAccessClient {
    fn from(config: BasicClientConfig) -> Self {
        Self {
            rollup_height: config.rollup_height,
            client: NodeClient::new_unchecked(&config.url),
        }
    }
}

impl HttpStorageAccessClient {
    pub async fn get_value(&self, item: u64) -> Option<String> {
        let rollup_height_param = if let Some(rollup_height) = self.rollup_height {
            format!("?rollup_height={rollup_height}")
        } else {
            String::new()
        };

        self.client
            .query_rest_endpoint::<MapResponse<String>>(&format!(
                "/modules/access-pattern/state/values/items/{item}{rollup_height_param}"
            ))
            .await
            .map(|r| r.value)
            .ok()
    }

    pub async fn get_begin_hook(&self, item: u64) -> Option<HooksConfig> {
        let rollup_height_param = if let Some(rollup_height) = self.rollup_height {
            format!("?rollup_height={rollup_height}")
        } else {
            String::new()
        };

        self.client
            .query_rest_endpoint::<MapResponse<HooksConfig>>(&format!(
                "/modules/access-pattern/state/pre-hook/items/{item}{rollup_height_param}"
            ))
            .await
            .map(|r| r.value)
            .ok()
    }

    pub async fn get_end_hook(&self, item: u64) -> Option<HooksConfig> {
        let rollup_height_param = if let Some(rollup_height) = self.rollup_height {
            format!("?rollup_height={rollup_height}")
        } else {
            String::new()
        };

        self.client
            .query_rest_endpoint::<MapResponse<HooksConfig>>(&format!(
                "/modules/access-pattern/state/post-hook/items/{item}{rollup_height_param}"
            ))
            .await
            .map(|r| r.value)
            .ok()
    }

    pub async fn get_admin<S: Spec>(&self) -> Option<S::Address> {
        let rollup_height_param = if let Some(rollup_height) = self.rollup_height {
            format!("?rollup_height={rollup_height}")
        } else {
            String::new()
        };

        let response = self
            .client
            .query_rest_endpoint::<ValueResponse<S::Address>>(&format!(
                "/modules/access-pattern/state/admin{rollup_height_param}"
            ))
            .await
            .unwrap();

        response.value
    }
}

#[derive(Debug, Deserialize)]
struct ValueResponse<T> {
    value: Option<T>,
}

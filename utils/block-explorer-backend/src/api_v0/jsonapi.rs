use std::collections::HashMap;
use std::sync::Arc;

use crate::Config;

/// Helpers for {JSON:API}.
/// See: <https://jsonapi.org/>.

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseObject<T> {
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub links: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseObjectData<T>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorObject>,
}

#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum ResponseObjectData<T> {
    Single(ResourceObject<T>),
    Many(Vec<ResourceObject<T>>),
}

impl<T> From<ResourceObject<T>> for ResponseObjectData<T> {
    fn from(resource: ResourceObject<T>) -> Self {
        Self::Single(resource)
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ResourceObject<T, Id = String> {
    #[serde(rename = "type")]
    pub resoure_type: &'static str,
    pub id: Id,
    pub attributes: T,
}

/// Many other fields are available, but we don't need them for now.
/// See: <https://jsonapi.org/format/#error-objects>.
#[derive(Debug, serde::Serialize)]
pub struct ErrorObject {
    pub status: i32,
    pub title: String,
    pub details: Option<String>,
}

pub struct Links {
    config: Arc<Config>,
    links: HashMap<String, String>,
}

impl Links {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            links: HashMap::new(),
        }
    }

    pub fn add(&mut self, name: impl ToString, path: impl AsRef<str>) {
        let mut url = self.config.base_url.clone();
        url.push_str(path.as_ref());
        self.links.insert(name.to_string(), url);
    }

    pub fn links(self) -> HashMap<String, String> {
        self.links
    }
}

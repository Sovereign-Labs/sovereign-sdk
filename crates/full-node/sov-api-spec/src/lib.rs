pub mod client;

pub use client::*;

const RAW_YAML_SPEC: &str = include_str!("../openapi-v3.yaml");

/// Returns parsed [`openapiv3::OpenAPI`] for Ledger JSON API.
/// Performs clone of the whole spec, so might be slow.
pub fn open_api_v3_spec() -> openapiv3::OpenAPI {
    static OPENAPI_SPEC_V3: std::sync::OnceLock<openapiv3::OpenAPI> = std::sync::OnceLock::new();
    OPENAPI_SPEC_V3
        .get_or_init(|| sov_modules_api::prelude::serde_yaml::from_str(RAW_YAML_SPEC).unwrap())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_is_valid() {
        let _spec = open_api_v3_spec();
    }
}

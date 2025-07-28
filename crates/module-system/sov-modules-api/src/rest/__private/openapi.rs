use std::collections::HashMap;
use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;

use heck::ToSnakeCase;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;
use sov_rollup_interface::common::SlotNumber;
use sov_state::{CompileTimeNamespace, StateCodec, StateItemCodec};
pub use utoipa::openapi::OpenApi;
use utoipa::openapi::PathItem;

use super::StateItemInfo;
use crate::containers::map::NamespacedStateMap;
use crate::containers::value::NamespacedStateValue;
use crate::containers::vec::NamespacedStateVec;
use crate::VersionedStateValue;

const OPENAPI_TEMPLATE: &str = include_str!("openapi-templates/runtime-base.yaml");

type OpenApiPaths = Vec<(String, serde_json::Value)>;

/// This should ideally be a reusable parameter defined in `#/parameters`, but
/// [`utoipa::openapi::path::Parameter`] doesn't support `$ref` at the time of
/// writing.
fn rollup_height_param() -> utoipa::openapi::path::Parameter {
    serde_json::from_value(json!({
        "name": "rollup_height",
        "in": "query",
        "description": "The height of the rollup to query. If neither slot_number nor rollup_height is provided, the rollup head is used.",
        "required": false,
        "schema": {
            "type": "integer",
            "minimum": 0,
        }
    }))
    .unwrap()
}

/// This should ideally be a reusable parameter defined in `#/parameters`, but
/// [`utoipa::openapi::path::Parameter`] doesn't support `$ref` at the time of
/// writing.
fn slot_number_param() -> utoipa::openapi::path::Parameter {
    serde_json::from_value(json!({
        "name": "slot_number",
        "in": "query",
        "description": "The slot number of the rollup to query. If neither slot_number nor rollup_height is provided, the rollup head is used.",
        "required": false,
        "schema": {
            "type": "integer",
            "minimum": 0,
        }
    }))
    .unwrap()
}

/// The OpenAPI paths specification for
/// [`StateValue`](crate::containers::StateValue).
pub fn state_value_paths(module_name: &str, field_name: &str) -> OpenApiPaths {
    vec![(
        "".to_string(),
        json!({
            "get": {
                "summary": "Get the value of a StateValue.",
                "operationId": format!("{}_{}_get_state_value", module_name.to_snake_case(), field_name),
                "tags": [module_name],
                "parameters": [rollup_height_param(), slot_number_param()],
                "responses": {
                    "200": {
                        "$ref": "#/components/responses/StateValueResponse"
                    },
                    "404": {
                        "$ref": "#/components/responses/StateNotFound"
                    }
                }
            }
        }),
    )]
}

/// The OpenAPI paths specification for
/// [`StateMap`](crate::containers::StateMap).
pub fn state_map_paths(module_name: &str, field_name: &str) -> OpenApiPaths {
    vec![
        (
            "".to_string(),
            json!({
                "get": {
                    "summary": "Get general information about a `StateMap`.",
                    "operationId": format!("{}_{}_get_state_map_info", module_name.to_snake_case(), field_name),
                    "tags": [module_name],
                    "parameters": [rollup_height_param(), slot_number_param()],
                    "responses": {
                        "200": {
                            "$ref": "#/components/responses/StateMapInfoResponse"
                        },
                        "400": {
                            "$ref": "#/components/responses/BadRequestResponse"
                        }
                    }
                }
            }),
        ),
        (
            "/items/{key}".to_string(),
            json!({
                "get": {
                    "summary": "Get the value of a StateMap element.",
                    "operationId": format!("{}_{}_get_state_map_element", module_name.to_snake_case(), field_name),
                    "tags": [module_name],
                    "parameters": [
                        {
                            "name": "key",
                            "in": "path",
                            "required": true,
                            "schema": {
                                "type": "string",
                            }
                        },
                        rollup_height_param(),
                        slot_number_param(),
                    ],
                    "responses": {
                        "200": {
                            "$ref": "#/components/responses/StateMapElementResponse"
                        },
                        "400": {
                            "$ref": "#/components/responses/BadRequestResponse"
                        },
                        "404": {
                            "$ref": "#/components/responses/StateNotFound"
                        }
                    }
                }
            }),
        ),
    ]
}

/// The OpenAPI paths specification for
/// [`StateVec`](crate::containers::StateVec).
pub fn state_vec_paths(module_name: &str, field_name: &str) -> OpenApiPaths {
    vec![
        (
            "".to_string(),
            json!({
                "get": {
                    "summary": "Get general information about a `StateVec`, including its length.",
                    "operationId": format!("{}_{}_get_state_vec_info", module_name.to_snake_case(), field_name),
                    "tags": [module_name],
                    "parameters": [rollup_height_param(), slot_number_param()],
                    "responses": {
                        "200": {
                            "$ref": "#/components/responses/StateVecInfoResponse"
                        }
                    }
                }
            }),
        ),
        (
            "/items/{index}".to_string(),
            json!({
                "get": {
                    "summary": "Get the value of a `StateVec` element.",
                    "operationId": format!("{}_{}_get_state_vec_element", module_name.to_snake_case(), field_name),
                    "tags": [module_name],
                    "parameters": [
                        {
                            "name": "index",
                            "in": "path",
                            "required": true,
                            "schema": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": null
                            }
                        },
                        rollup_height_param(),
                        slot_number_param(),
                    ],
                    "responses": {
                        "200": {
                            "$ref": "#/components/responses/StateVecElementResponse"
                        },
                         "400": {
                            "$ref": "#/components/responses/BadRequestResponse"
                        },
                        "404": {
                            "$ref": "#/components/responses/StateNotFound"
                        }
                    }
                }
            }),
        ),
    ]
}

pub fn runtime_spec(module_specs: HashMap<String, OpenApi>) -> OpenApi {
    let mut runtime_spec: OpenApi = serde_yaml::from_str(OPENAPI_TEMPLATE).unwrap();

    // Because: https://github.com/juhaku/utoipa/issues/972
    for runtime_path in runtime_spec.paths.paths.values_mut() {
        runtime_path.extensions = None;
    }

    for (module_name, mut module_spec) in module_specs {
        let old_paths = std::mem::take(&mut module_spec.paths);
        for (path, mut path_item) in old_paths.paths {
            let runtime_path = format!("/modules/{module_name}{path}");
            // Because: https://github.com/juhaku/utoipa/issues/972
            path_item.extensions = None;
            module_spec.paths.paths.insert(runtime_path, path_item);
        }
        runtime_spec.merge(module_spec);
    }

    runtime_spec
}

#[derive(derivative::Derivative)]
#[derivative(Clone(bound = ""))]
pub struct StateItemOpenApiSpecImpl<T> {
    pub state_item_info: StateItemInfo,
    pub phantom: PhantomData<T>,
}

pub trait StateItemOpenApiSpec {
    fn state_item_open_api(&self, module_name: &str) -> OpenApi;
}

impl<T> StateItemOpenApiSpec for &T {
    fn state_item_open_api(&self, _module_name: &str) -> OpenApi {
        OpenApi::default()
    }
}

impl<N, T, Codec> StateItemOpenApiSpec
    for StateItemOpenApiSpecImpl<NamespacedStateValue<N, T, Codec>>
where
    N: CompileTimeNamespace,
    T: Serialize + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T>,
{
    fn state_item_open_api(&self, module_name: &str) -> OpenApi {
        let paths = state_value_paths(module_name, &self.state_item_info.name);
        spec_from_json_paths(paths)
    }
}

impl<N, T, Codec> StateItemOpenApiSpec for StateItemOpenApiSpecImpl<NamespacedStateVec<N, T, Codec>>
where
    N: CompileTimeNamespace,
    T: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<u64>,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<u64>,
{
    fn state_item_open_api(&self, module_name: &str) -> OpenApi {
        let paths = state_vec_paths(module_name, &self.state_item_info.name);
        spec_from_json_paths(paths)
    }
}

impl<N, K, V, Codec> StateItemOpenApiSpec
    for StateItemOpenApiSpecImpl<NamespacedStateMap<N, K, V, Codec>>
where
    N: CompileTimeNamespace,
    K: Serialize + DeserializeOwned + FromStr + Display + Clone + Send + Sync + 'static,
    V: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
{
    fn state_item_open_api(&self, module_name: &str) -> OpenApi {
        let paths = state_map_paths(module_name, &self.state_item_info.name);
        spec_from_json_paths(paths)
    }
}

impl<V, Codec> StateItemOpenApiSpec for StateItemOpenApiSpecImpl<VersionedStateValue<V, Codec>>
where
    V: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
    Codec::ValueCodec: StateItemCodec<V>,
{
    fn state_item_open_api(&self, module_name: &str) -> OpenApi {
        let paths = state_value_paths(module_name, &self.state_item_info.name);
        spec_from_json_paths(paths)
    }
}

fn spec_from_json_paths(paths: OpenApiPaths) -> OpenApi {
    let mut item_spec = OpenApi::default();
    for (field_name, raw_path) in paths {
        let path_item: PathItem = serde_json::from_value(raw_path).unwrap();
        item_spec.paths.paths.insert(field_name, path_item);
    }
    item_spec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_template_is_valid() {
        let _: OpenApi = serde_yaml::from_str(OPENAPI_TEMPLATE).unwrap();
    }
}

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
use utoipa::openapi::{PathItem, Response};
use utoipa::ToSchema;

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
    state_value_paths_with_response(module_name, field_name, None)
}

/// The OpenAPI paths specification for
/// [`StateMap`](crate::containers::StateMap).
pub fn state_map_paths(module_name: &str, field_name: &str) -> OpenApiPaths {
    state_map_paths_with_response(module_name, field_name, None)
}

/// The OpenAPI paths specification for
/// [`StateVec`](crate::containers::StateVec).
pub fn state_vec_paths(module_name: &str, field_name: &str) -> OpenApiPaths {
    state_vec_paths_with_response(module_name, field_name, None)
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

pub trait CustomStateItemPath {
    fn generate_custom_path(
        &self,
        module_name: &str,
        field_name: &str,
    ) -> Option<(OpenApiPaths, String, Response)>;
}

pub trait StateItemPaths {
    fn state_item_paths(&self, module_name: &str, field_name: &str) -> Option<OpenApiPaths>;
}

impl<T> StateItemPaths for &T {
    fn state_item_paths(&self, _module_name: &str, _field_name: &str) -> Option<OpenApiPaths> {
        None
    }
}

impl<T> CustomStateItemPath for &T {
    fn generate_custom_path(
        &self,
        _module_name: &str,
        _field_name: &str,
    ) -> Option<(OpenApiPaths, String, Response)> {
        None
    }
}

impl<N, T, Codec> CustomStateItemPath
    for StateItemOpenApiSpecImpl<NamespacedStateValue<N, T, Codec>>
where
    N: CompileTimeNamespace,
    T: for<'a> ToSchema<'a> + Serialize + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T>,
{
    /// Generate custom StateMap response for ToSchema types
    fn generate_custom_path(
        &self,
        module_name: &str,
        field_name: &str,
    ) -> Option<(OpenApiPaths, String, Response)> {
        let (name, _) = <T as ToSchema<'_>>::schema();
        let custom_element_response = format!("{name}StateItemResponse");

        Some((
            state_value_paths_with_response(
                module_name,
                field_name,
                Some(&custom_element_response),
            ),
            custom_element_response,
            make_simple_custom_response::<T>(""),
        ))
    }
}

impl<N, K, V, Codec> CustomStateItemPath
    for StateItemOpenApiSpecImpl<NamespacedStateMap<N, K, V, Codec>>
where
    N: CompileTimeNamespace,
    K: Serialize + DeserializeOwned + FromStr + Display + Clone + Send + Sync + 'static,
    V: for<'a> ToSchema<'a> + Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
{
    /// Generate custom StateMap response for ToSchema types
    fn generate_custom_path(
        &self,
        module_name: &str,
        field_name: &str,
    ) -> Option<(OpenApiPaths, String, Response)> {
        let (name, _) = <V as ToSchema<'_>>::schema();
        let custom_element_response = format!("{name}StateMapElementResponse");

        Some((
            state_map_paths_with_response(module_name, field_name, Some(&custom_element_response)),
            custom_element_response,
            make_custom_response_map::<V>(""),
        ))
    }
}

impl<V, Codec> StateItemPaths for StateItemOpenApiSpecImpl<VersionedStateValue<V, Codec>>
where
    V: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
    fn state_item_paths(&self, module_name: &str, field_name: &str) -> Option<OpenApiPaths> {
        Some(state_value_paths(module_name, field_name))
    }
}

impl<N, T, Codec> CustomStateItemPath for StateItemOpenApiSpecImpl<NamespacedStateVec<N, T, Codec>>
where
    N: CompileTimeNamespace,
    T: Serialize + Send + Sync + 'static + for<'a> ToSchema<'a>,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<u64>,
    Codec::KeyCodec: StateItemCodec<u64>,
{
    /// Generate custom StateMap response for ToSchema types
    fn generate_custom_path(
        &self,
        module_name: &str,
        field_name: &str,
    ) -> Option<(OpenApiPaths, String, Response)> {
        let (name, _) = <T as ToSchema<'_>>::schema();
        let custom_element_response = format!("{name}StateVecElementResponse");

        Some((
            state_vec_paths_with_response(module_name, field_name, Some(&custom_element_response)),
            custom_element_response,
            make_custom_response_vec::<T>(""),
        ))
    }
}

impl<V, Codec> CustomStateItemPath for StateItemOpenApiSpecImpl<VersionedStateValue<V, Codec>>
where
    V: Serialize + Clone + Send + Sync + 'static + for<'a> ToSchema<'a>,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
    /// Generate custom StateMap response for ToSchema types
    fn generate_custom_path(
        &self,
        module_name: &str,
        field_name: &str,
    ) -> Option<(OpenApiPaths, String, Response)> {
        let (name, _) = <V as ToSchema<'_>>::schema();
        let custom_element_response = format!("{name}StateItemResponse");

        Some((
            state_value_paths_with_response(
                module_name,
                field_name,
                Some(&custom_element_response),
            ),
            custom_element_response,
            make_simple_custom_response::<V>(""),
        ))
    }
}

impl<N, K, V, Codec> StateItemPaths for StateItemOpenApiSpecImpl<NamespacedStateMap<N, K, V, Codec>>
where
    N: CompileTimeNamespace,
    K: Serialize + DeserializeOwned + FromStr + Display + Clone + Send + Sync + 'static,
    V: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
{
    fn state_item_paths(&self, module_name: &str, field_name: &str) -> Option<OpenApiPaths> {
        Some(state_map_paths(module_name, field_name))
    }
}

impl<N, T, Codec> StateItemPaths for StateItemOpenApiSpecImpl<NamespacedStateVec<N, T, Codec>>
where
    N: CompileTimeNamespace,
    T: Serialize + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<u64>,
    Codec::KeyCodec: StateItemCodec<u64>,
{
    fn state_item_paths(&self, module_name: &str, field_name: &str) -> Option<OpenApiPaths> {
        Some(state_vec_paths(module_name, field_name))
    }
}

impl<N, T, Codec> StateItemPaths for StateItemOpenApiSpecImpl<NamespacedStateValue<N, T, Codec>>
where
    N: CompileTimeNamespace,
    T: Serialize + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T>,
{
    fn state_item_paths(&self, module_name: &str, field_name: &str) -> Option<OpenApiPaths> {
        Some(state_value_paths(module_name, field_name))
    }
}

/// The OpenAPI paths specification for
/// [`StateValue`](crate::containers::StateValue) with optional custom response type.
pub fn state_value_paths_with_response(
    module_name: &str,
    field_name: &str,
    custom_response: Option<&str>,
) -> OpenApiPaths {
    let response_ref = custom_response
        .map(|r| format!("#/components/responses/{r}"))
        .unwrap_or_else(|| "#/components/responses/StateValueResponse".to_string());

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
                        "$ref": response_ref
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
/// [`StateMap`](crate::containers::StateMap) with optional custom response type.
pub fn state_map_paths_with_response(
    module_name: &str,
    field_name: &str,
    custom_element_response: Option<&str>,
) -> OpenApiPaths {
    let element_response_ref = custom_element_response
        .map(|r| format!("#/components/responses/{r}"))
        .unwrap_or_else(|| "#/components/responses/StateMapElementResponse".to_string());
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
                            "$ref": element_response_ref
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
/// [`StateVec`](crate::containers::StateVec) with optional custom response type.
pub fn state_vec_paths_with_response(
    module_name: &str,
    field_name: &str,
    custom_element_response: Option<&str>,
) -> OpenApiPaths {
    let element_response_ref = custom_element_response
        .map(|r| format!("#/components/responses/{r}"))
        .unwrap_or_else(|| "#/components/responses/StateVecElementResponse".to_string());
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
                            "$ref": element_response_ref
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

/// Add a simple custom response to the OpenAPI spec
fn make_simple_custom_response<'a, 'resp, T: ToSchema<'a>>(description: &str) -> Response {
    // Create a simple response with a basic schema
    let response = utoipa::openapi::ResponseBuilder::new()
        .description(description)
        .content(
            "application/json",
            utoipa::openapi::ContentBuilder::new()
                .schema(utoipa::openapi::RefOr::T(utoipa::openapi::Schema::Object(
                    utoipa::openapi::ObjectBuilder::new()
                        .description(Some(description.to_string()))
                        .property("value", T::schema().1)
                        .required("value")
                        .build(),
                )))
                .build(),
        )
        .build();

    response
}

/// Add a simple custom response to the OpenAPI spec
fn make_custom_response_map<'a, 'resp, T: ToSchema<'a>>(description: &str) -> Response {
    // Create a simple response with a basic schema
    let response = utoipa::openapi::ResponseBuilder::new()
        .description(description)
        .content(
            "application/json",
            utoipa::openapi::ContentBuilder::new()
                .schema(utoipa::openapi::RefOr::T(utoipa::openapi::Schema::Object(
                    utoipa::openapi::ObjectBuilder::new()
                        .description(Some(description.to_string()))
                        .property(
                            "key",
                            utoipa::openapi::ObjectBuilder::new()
                                .schema_type(utoipa::openapi::SchemaType::String),
                        )
                        .required("key")
                        .property("value", T::schema().1)
                        .required("value")
                        .build(),
                )))
                .build(),
        )
        .build();

    response
}

/// Add a simple custom response to the OpenAPI spec
fn make_custom_response_vec<'a, 'resp, T: ToSchema<'a>>(description: &str) -> Response {
    // Create a simple response with a basic schema
    let response = utoipa::openapi::ResponseBuilder::new()
        .description(description)
        .content(
            "application/json",
            utoipa::openapi::ContentBuilder::new()
                .schema(utoipa::openapi::RefOr::T(utoipa::openapi::Schema::Object(
                    utoipa::openapi::ObjectBuilder::new()
                        .description(Some(description.to_string()))
                        .property(
                            "index",
                            utoipa::openapi::ObjectBuilder::new()
                                .schema_type(utoipa::openapi::SchemaType::Integer),
                        )
                        .required("index")
                        .property("value", T::schema().1)
                        .required("value")
                        .build(),
                )))
                .build(),
        )
        .build();

    response
}

pub fn add_simple_custom_response(spec: &mut OpenApi, response_name: &str, response: Response) {
    // Ensure components exist
    if spec.components.is_none() {
        spec.components = Some(utoipa::openapi::Components::default());
    }

    let components = spec.components.as_mut().unwrap();

    // Add the response to components/responses
    components
        .responses
        .insert(response_name.to_string(), response.into());
}

pub fn spec_from_json_paths(paths: OpenApiPaths) -> OpenApi {
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

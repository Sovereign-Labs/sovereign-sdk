use std::collections::HashMap;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use reqwest::{Client, StatusCode};
use serde_json::json;
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::hooks::TxHooks;
use sov_modules_api::rest::{ApiState, HasRestApi};
use sov_modules_api::{
    ConcurrentStateCheckpoint, Context, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec,
    StateCheckpoint, StateValue, TxState,
};
use sov_test_utils::TestSpec;
use unwrap_infallible::UnwrapInfallible;
use utoipa::openapi::path::ParameterIn;
use utoipa::openapi::PathItemType;

#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct Foo {
    i: u64,
    j: u64,
}

impl FromStr for Foo {
    type Err = std::convert::Infallible;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        Ok(Self { i: 0, j: 0 })
    }
}

impl Display for Foo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.i)
    }
}

#[derive(Clone, ModuleInfo, ModuleRestApi)]
#[allow(dead_code)]
pub struct MyModule<S: Spec, D>
where
    D: std::hash::Hash
        + Clone
        + borsh::BorshSerialize
        + borsh::BorshDeserialize
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + std::str::FromStr
        + std::fmt::Display
        + 'static,
{
    #[id]
    pub id: ModuleId,

    // Normal values
    #[state]
    pub value: ::sov_modules_api::StateValue<D>,
    #[state]
    pub another_value: StateValue<String>,
    #[state]
    pub mapping: sov_modules_api::StateMap<D, D>,
    #[state]
    pub list: sov_modules_api::StateVec<D>,

    // Skipped values, because missing serde serialization
    #[state]
    pub skipped_value: StateValue<Foo>,
    #[state]
    pub skipped_mapping: sov_modules_api::StateMap<Foo, Foo>,
    #[state]
    pub key_skipped_mapping: sov_modules_api::StateMap<Foo, D>,
    #[state]
    pub value_skipped_mapping: sov_modules_api::StateMap<D, Foo>,

    #[state]
    pub skipped_list: sov_modules_api::StateVec<Foo>,
    // Explicitly skipped value
    #[rest_api(skip)]
    #[state]
    pub explicitly_skipped_value: ::sov_modules_api::StateValue<D>,
    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec, D> Module for MyModule<S, D>
where
    D: std::hash::Hash
        + Clone
        + borsh::BorshSerialize
        + borsh::BorshDeserialize
        + serde::Serialize
        + serde::de::DeserializeOwned
        + std::str::FromStr
        + std::fmt::Display
        + Send
        + Sync
        + 'static,
{
    type Spec = S;
    type Config = ();
    type CallMessage = ();
    type Event = ();
    type Error = anyhow::Error;

    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<Self::Spec>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Default, sov_modules_api::macros::RuntimeRestApi)]
pub struct MyRuntime<S: Spec> {
    my_foo_module: MyModule<S, u32>,
}

impl<S: Spec> TxHooks for MyRuntime<S> {
    type Spec = S;
}

#[tokio::test(flavor = "multi_thread")]
async fn rest_api_routes() {
    let _module_name = "my-foo-module";
    let _data: u32 = 1200;

    let mut values = HashMap::new();
    values.insert("{key}", _data.to_string());
    values.insert("{index}", "0".to_string());
    values.insert("{moduleName}", _module_name.to_string());

    let storage_manager = sov_test_utils::storage::SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    let (_sender, receiver) =
        tokio::sync::watch::channel(Arc::new(ConcurrentStateCheckpoint::from_state_checkpoint(
            StateCheckpoint::new(storage, &MockKernel::<TestSpec>::default(), None),
        )));
    let runtime = MyRuntime::<TestSpec>::default();
    let state = ApiState::build(
        Arc::new(()),
        receiver,
        Arc::new(MockKernel::default()),
        None,
    );

    let router = runtime.rest_api(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest_address = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = Client::new();

    let spec = runtime.openapi_spec().unwrap();

    let base_path = spec
        .servers
        .clone()
        .unwrap()
        .first()
        .unwrap()
        .url
        .replace("localhost:12346", rest_address.to_string().as_str());

    let serialized_spec = spec.to_json().unwrap();
    let deserialized: openapiv3::OpenAPI =
        serde_json::from_str(&serialized_spec).expect("Runtime schema is bad");

    assert_eq!(deserialized.paths.paths.len(), spec.paths.paths.len());

    // 1. Root
    // 2. Module details
    // 3-4. State values
    // 5-6. Vec: info and item
    // 7-9. Map: info, collection, and item
    let expected_paths_count = 9;
    println!("spec.paths.paths: {:#?}", spec.paths.paths);
    assert_eq!(expected_paths_count, spec.paths.paths.len());
    for (path, item) in spec.paths.paths {
        let get_operation = match item.operations.get(&PathItemType::Get) {
            None => {
                continue;
            }
            Some(o) => o,
        };

        let path_parameters = get_operation
            .parameters
            .as_ref()
            .map(|parameters| {
                parameters
                    .iter()
                    .filter(|&param| param.parameter_in == ParameterIn::Path)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let brace_count = path.chars().filter(|&c| c == '{').count();
        assert_eq!(
            path_parameters.len(),
            brace_count,
            "Missing parameter declaration in path {path} {path_parameters:?}"
        );

        let mut path = path;
        if !path_parameters.is_empty() {
            for (key, value) in values.iter() {
                path = path.replace(key, value);
            }
        }

        let url = format!("{base_path}{path}");
        let response = client
            .get(&url)
            .send()
            .await
            .expect("Failed querying router");

        let status = response.status();
        // Root resource is trimmed by top level router, but here it will get HTTP 404.
        let success_condition = if path_parameters.is_empty() {
            status.is_success()
        } else {
            !status.is_server_error()
        };
        assert!(success_condition, "Failed querying URL {url} | {status}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn state_map_items_route_lists_current_checkpoint_state() {
    let module_name = "my-foo-module";
    let mut runtime = MyRuntime::<TestSpec>::default();
    let storage_manager = sov_test_utils::storage::SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    let mut checkpoint = StateCheckpoint::new(storage, &MockKernel::<TestSpec>::default(), None);

    runtime
        .my_foo_module
        .mapping
        .set(&1, &10, &mut checkpoint)
        .unwrap_infallible();
    runtime
        .my_foo_module
        .mapping
        .set(&2, &20, &mut checkpoint)
        .unwrap_infallible();

    let (_sender, receiver) = tokio::sync::watch::channel(Arc::new(
        ConcurrentStateCheckpoint::from_state_checkpoint(checkpoint),
    ));
    let state = ApiState::build(
        Arc::new(()),
        receiver,
        Arc::new(MockKernel::default()),
        None,
    );
    let router = runtime.rest_api(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest_address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = Client::new();
    let base_url = format!("http://{rest_address}/modules/{module_name}/state/mapping/items");

    let response = client.get(&base_url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        body,
        json!({
            "items": [
                { "key": 1, "value": 10 },
                { "key": 2, "value": 20 }
            ],
            "next_cursor": null
        })
    );

    let first_page = client
        .get(&base_url)
        .query(&[("page", "first"), ("page[size]", "1")])
        .send()
        .await
        .unwrap();
    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page_body: serde_json::Value = first_page.json().await.unwrap();
    assert_eq!(
        first_page_body,
        json!({
            "items": [{ "key": 1, "value": 10 }],
            "next_cursor": "1"
        })
    );

    let second_page = client
        .get(&base_url)
        .query(&[("page", "next"), ("page[cursor]", "1"), ("page[size]", "1")])
        .send()
        .await
        .unwrap();
    assert_eq!(second_page.status(), StatusCode::OK);
    let second_page_body: serde_json::Value = second_page.json().await.unwrap();
    assert_eq!(
        second_page_body,
        json!({
            "items": [{ "key": 2, "value": 20 }],
            "next_cursor": null
        })
    );

    let exact_page = client
        .get(&base_url)
        .query(&[("page", "first"), ("page[size]", "2")])
        .send()
        .await
        .unwrap();
    assert_eq!(exact_page.status(), StatusCode::OK);
    let exact_page_body: serde_json::Value = exact_page.json().await.unwrap();
    assert_eq!(
        exact_page_body,
        json!({
            "items": [
                { "key": 1, "value": 10 },
                { "key": 2, "value": 20 }
            ],
            "next_cursor": null
        })
    );

    let historical_response = client
        .get(&base_url)
        .query(&[("page", "first"), ("rollup_height", "0")])
        .send()
        .await
        .unwrap();
    assert_eq!(historical_response.status(), StatusCode::NOT_IMPLEMENTED);
}

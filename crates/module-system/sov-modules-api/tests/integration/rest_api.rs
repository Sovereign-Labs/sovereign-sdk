use std::collections::HashMap;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use reqwest::Client;
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::hooks::TxHooks;
use sov_modules_api::rest::{utils::ErrorObject, ApiState, HasRestApi};
use sov_modules_api::{
    ConcurrentStateCheckpoint, Context, EventEpoch, EventFrontier, Module, ModuleId, ModuleInfo,
    ModuleRestApi, Spec, StateCheckpoint, StateValue, TxState,
};
use sov_test_utils::TestSpec;
use utoipa::openapi::path::ParameterIn;

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
    <D as std::str::FromStr>::Err: std::fmt::Display,
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
    <D as std::str::FromStr>::Err: std::fmt::Display,
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
            StateCheckpoint::new(storage, &MockKernel::<TestSpec>::default()),
        )));
    let runtime = MyRuntime::<TestSpec>::default();
    let state = ApiState::build(
        Arc::new(()),
        receiver,
        Arc::new(MockKernel::default()),
        None,
        sov_shutdown::PrimaryShutdownController::new(),
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
    let spec_json: serde_json::Value =
        serde_json::from_str(&serialized_spec).expect("Runtime schema JSON is bad");
    let key_description = spec_json
        .pointer("/paths/~1modules~1my-foo-module~1state~1mapping~1items~1{key}/get/parameters/0/description")
        .and_then(serde_json::Value::as_str)
        .expect("StateMap key parameter description is missing");
    assert!(key_description.contains("FromStr"));
    assert!(key_description.contains("Display"));

    let cursor_description = spec_json
        .pointer(
            "/paths/~1modules~1my-foo-module~1state~1mapping~1items/get/parameters/2/description",
        )
        .and_then(serde_json::Value::as_str)
        .expect("StateMap cursor parameter description is missing");
    assert!(cursor_description.contains("Display"));

    let next_cursor_description = spec_json
        .pointer("/paths/~1modules~1my-foo-module~1state~1mapping~1items/get/responses/200/content/application~1json/schema/properties/next_cursor/description")
        .and_then(serde_json::Value::as_str)
        .expect("StateMap next_cursor description is missing");
    assert!(next_cursor_description.contains("Display"));

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
        let get_operation = match &item.get {
            None => {
                continue;
            }
            Some(o) => o,
        };

        let path_parameters = get_operation
            .parameters
            .as_ref()
            .map(|parameters: &Vec<_>| {
                parameters
                    .iter()
                    .filter(|param| param.parameter_in == ParameterIn::Path)
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

    let invalid_key_url =
        format!("{base_path}/modules/{_module_name}/state/mapping/items/not-a-u32");
    let response = client
        .get(&invalid_key_url)
        .send()
        .await
        .expect("Failed querying router");
    assert_eq!(reqwest::StatusCode::BAD_REQUEST, response.status());

    let error: ErrorObject = response
        .json()
        .await
        .expect("Invalid key response is not an ErrorObject");
    assert_eq!("Invalid key", error.message);
    let error_details = error
        .details
        .get("error")
        .and_then(serde_json::Value::as_str)
        .expect("Invalid key response does not include parse error details");
    assert!(error_details.contains("not-a-u32"));
    assert!(error_details.contains("invalid digit found in string"));
}

/// Multi-byte, asymmetric values, so a byte-order mistake anywhere would show up.
const NEXT_EVENT_NUMBER: u64 = 0x1234_5678;
const EPOCH: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;

/// Serves `MyRuntime`'s REST API and returns a client, its base URL, and the checkpoint
/// sender, which the caller must keep alive for as long as it makes requests.
async fn serve_rest_api(
    next_event_number: Option<u64>,
) -> (
    Client,
    String,
    tokio::sync::watch::Sender<Arc<ConcurrentStateCheckpoint<TestSpec>>>,
) {
    let storage_manager = sov_test_utils::storage::SimpleStorageManager::new();
    let storage = storage_manager.create_storage();

    let mut checkpoint = ConcurrentStateCheckpoint::from_state_checkpoint(StateCheckpoint::new(
        storage,
        &MockKernel::<TestSpec>::default(),
    ));
    if let Some(next_event_number) = next_event_number {
        checkpoint = checkpoint.with_event_frontier(EventFrontier::new(
            EventEpoch::new(EPOCH),
            next_event_number,
        ));
    }

    let (sender, receiver) = tokio::sync::watch::channel(Arc::new(checkpoint));

    let router = MyRuntime::<TestSpec>::default().rest_api(ApiState::build(
        Arc::new(()),
        receiver,
        Arc::new(MockKernel::default()),
        None,
        sov_shutdown::PrimaryShutdownController::new(),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest_address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    (Client::new(), format!("http://{rest_address}"), sender)
}

const LAST_EVENT_NUMBER_HEADER: &str = "x-sov-last-event-number";
const EVENT_EPOCH_HEADER: &str = "x-sov-event-epoch";

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn state_reads_report_the_last_event_number() {
    let (client, base, _sender) = serve_rest_api(Some(NEXT_EVENT_NUMBER)).await;

    let response = client
        .get(format!("{base}/modules/my-foo-module/state/value"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        header(&response, LAST_EVENT_NUMBER_HEADER),
        Some((NEXT_EVENT_NUMBER - 1).to_string()),
        "A latest-state read must tell the client which events its reply already reflects"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn state_reads_report_the_event_epoch() {
    let (client, base, _sender) = serve_rest_api(Some(NEXT_EVENT_NUMBER)).await;

    let response = client
        .get(format!("{base}/modules/my-foo-module/state/value"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        header(&response, EVENT_EPOCH_HEADER),
        Some(format!("{EPOCH:032x}")),
        "Without the epoch a client cannot tell a reissued event number from a new one, so it          must be reported alongside the number"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn historical_state_reads_report_no_last_event_number() {
    let (client, base, _sender) = serve_rest_api(Some(NEXT_EVENT_NUMBER)).await;

    let response = client
        .get(format!(
            "{base}/modules/my-foo-module/state/value?rollup_height=0"
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(
        (
            header(&response, LAST_EVENT_NUMBER_HEADER),
            header(&response, EVENT_EPOCH_HEADER),
        ),
        (None, None),
        "The live frontier says nothing about a historical read, so it must not be reported"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn state_reads_report_no_event_number_when_the_producer_tracks_none() {
    let (client, base, _sender) = serve_rest_api(None).await;

    let response = client
        .get(format!("{base}/modules/my-foo-module/state/value"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        (
            header(&response, LAST_EVENT_NUMBER_HEADER),
            header(&response, EVENT_EPOCH_HEADER),
        ),
        (None, None),
        "A checkpoint whose producer does not track event numbers must not report a frontier"
    );
}

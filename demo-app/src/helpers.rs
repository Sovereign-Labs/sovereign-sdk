use serde::de::DeserializeOwned;
use sov_modules_api::{mocks::MockContext, DispatchQuery};
use sov_state::{mocks::MockStorageSpec, ProverStorage, WorkingSet};
use std::str;

use crate::runtime::Runtime;

pub(crate) fn check_query(
    runtime: &mut Runtime<MockContext>,
    query: Vec<u8>,
    expected_response: &str,
    storage: ProverStorage<MockStorageSpec>,
) {
    let module = Runtime::<MockContext>::decode_query(&query).unwrap();
    let query_response = runtime.dispatch_query(module, &mut WorkingSet::new(storage));

    let response = str::from_utf8(&query_response.response).unwrap();
    assert_eq!(response, expected_response)
}

pub(crate) fn query_and_deserialize<R: DeserializeOwned>(
    runtime: &mut Runtime<MockContext>,
    query: Vec<u8>,
    storage: ProverStorage<MockStorageSpec>,
) -> R {
    let module = Runtime::<MockContext>::decode_query(&query).unwrap();
    let query_response = runtime.dispatch_query(module, &mut WorkingSet::new(storage));
    serde_json::from_slice(&query_response.response).expect("Failed to deserialize response json")
}

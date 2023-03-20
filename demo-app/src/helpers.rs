use sov_modules_api::{mocks::MockContext, DispatchQuery};
use sov_state::{mocks::MockStorageSpec, ProverStorage, WorkingSet};
use std::str;

use crate::runtime::Runtime;

pub(crate) fn check_query(
    query: Vec<u8>,
    expected_response: &str,
    storage: ProverStorage<MockStorageSpec>,
) {
    let module = Runtime::<MockContext>::decode_query(&query).unwrap();
    let query_response = module.dispatch_query(&mut WorkingSet::new(storage));

    let response = str::from_utf8(&query_response.response).unwrap();
    assert_eq!(response, expected_response)
}

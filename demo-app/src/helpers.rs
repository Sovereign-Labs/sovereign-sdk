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
    match query_response {
        crate::runtime::RuntimeQueryResponse::Election(election_response) => {
            let response = str::from_utf8(&election_response.response).unwrap();
            assert_eq!(response, expected_response)
        }
        crate::runtime::RuntimeQueryResponse::ValueSetter(value_setter_response) => {
            let response = str::from_utf8(&value_setter_response.response).unwrap();
            assert_eq!(response, expected_response)
        }
        crate::runtime::RuntimeQueryResponse::Accounts(accounts_response) => {
            let response = str::from_utf8(&accounts_response.response).unwrap();
            assert_eq!(response, expected_response)
        }
    }
}

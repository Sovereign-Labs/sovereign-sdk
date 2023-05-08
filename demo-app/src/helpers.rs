use serde::de::DeserializeOwned;
use sov_modules_api::{default_context::DefaultContext, DispatchQuery};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

use crate::runtime::Runtime;

pub(crate) fn query_and_deserialize<R: DeserializeOwned>(
    runtime: &mut Runtime<DefaultContext>,
    query: Vec<u8>,
    storage: ProverStorage<DefaultStorageSpec>,
) -> R {
    let module = Runtime::<DefaultContext>::decode_query(&query).unwrap();
    let query_response = runtime.dispatch_query(module, &mut WorkingSet::new(storage));
    serde_json::from_slice(&query_response.response).expect("Failed to deserialize response json")
}

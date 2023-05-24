use crate::runtime::Runtime;
use borsh::BorshSerialize;
use serde::de::DeserializeOwned;
use sov_app_template::Batch;
use sov_modules_api::{default_context::DefaultContext, Address, DispatchQuery};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};
pub fn query_and_deserialize<R: DeserializeOwned>(
    runtime: &mut Runtime<DefaultContext>,
    query: Vec<u8>,
    storage: ProverStorage<DefaultStorageSpec>,
) -> R {
    let module = Runtime::<DefaultContext>::decode_query(&query).unwrap();
    let query_response = runtime.dispatch_query(module, &mut WorkingSet::new(storage));
    serde_json::from_slice(&query_response.response).expect("Failed to deserialize response json")
}

pub type TestBlob = sov_rollup_interface::mocks::TestBlob<Address>;

pub fn new_test_blob(batch: Batch, address: &[u8]) -> TestBlob {
    let address = Address::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    TestBlob::new(data, address)
}

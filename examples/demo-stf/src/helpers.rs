use borsh::BorshSerialize;
use serde::de::DeserializeOwned;
use sov_app_template::Batch;
use sov_modules_api::{default_context::DefaultContext, Address, DispatchQuery};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};
use sovereign_core::da::BlobTransactionTrait;

use crate::runtime::Runtime;

pub fn query_and_deserialize<R: DeserializeOwned>(
    runtime: &mut Runtime<DefaultContext>,
    query: Vec<u8>,
    storage: ProverStorage<DefaultStorageSpec>,
) -> R {
    let module = Runtime::<DefaultContext>::decode_query(&query).unwrap();
    let query_response = runtime.dispatch_query(module, &mut WorkingSet::new(storage));
    serde_json::from_slice(&query_response.response).expect("Failed to deserialize response json")
}

#[derive(
    Debug,
    Clone,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct TestBlob {
    address: Address,
    data: Vec<u8>,
}

impl BlobTransactionTrait for TestBlob {
    type Data = std::io::Cursor<Vec<u8>>;

    type Address = Address;

    fn sender(&self) -> Self::Address {
        self.address.clone()
    }

    fn data(&self) -> Self::Data {
        std::io::Cursor::new(self.data.clone())
    }
}

impl TestBlob {
    pub fn new(batch: Batch, address: &[u8]) -> Self {
        Self {
            address: TryInto::<Address>::try_into(address).unwrap(),
            data: batch.try_to_vec().unwrap(),
        }
    }
}

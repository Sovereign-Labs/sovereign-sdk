use borsh::BorshSerialize;
use sov_app_template::Batch;
use sov_modules_api::Address;
pub type TestBlob = sov_rollup_interface::mocks::TestBlob<Address>;

pub fn new_test_blob(batch: Batch, address: &[u8]) -> TestBlob {
    let address = Address::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    TestBlob::new(data, address)
}

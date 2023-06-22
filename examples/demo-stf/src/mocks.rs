use borsh::BorshSerialize;
use sov_modules_api::Address;
use sov_modules_stf_template::Batch;
use sov_rollup_interface::mocks::TestBlob;

pub fn new_test_blob(batch: Batch, address: &[u8]) -> TestBlob<Address> {
    let address = Address::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    TestBlob::new(data, address, [0; 32])
}

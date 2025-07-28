use std::env;

use sov_mock_da::MockBlob;
use sov_modules_api::BlobReaderTrait;
use sov_rollup_interface::common::HexString;
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::{TestUser, TEST_DEFAULT_USER_BALANCE};

use crate::stf_blueprint::operator::operator_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{create_blob, PriorityFeeBips, TxStatus, S};

#[test]
fn invalid_blobs_are_discarded() {
    // The BlobSelector will discard blobs becouse `MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE` is set to 0
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "0",
    );

    let priority_fee_bips = PriorityFeeBips::from_percentage(0);
    let reward_user = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    let (mut runner, users, _sequencer_account) = setup(reward_user, 2);

    let admin_account = &users[0];
    let not_admin_account = &users[1];

    let mock_blob = create_blob::<IntegTestRuntime<S>>(
        &[TxStatus::Success],
        priority_fee_bips,
        admin_account,
        not_admin_account,
        runner.config.sequencer_da_address,
    );

    let mock_blob_hash = mock_blob.hash();

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![mock_blob],
    };

    let (result, _) = runner.execute::<RelevantBlobs<MockBlob>>(blobs);

    // Check that the blob was discarded
    assert_eq!(result.discarded_blobs[0], HexString(mock_blob_hash.0));
    assert!(result.batch_receipts.is_empty());
}

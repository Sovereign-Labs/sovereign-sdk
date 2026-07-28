//! Tests for the hardcoded consensus exception in `apply_slot` that treats Celestia DA
//! block 10645809 as containing no rollup blobs.

use sov_mock_da::MockBlob;
use sov_rollup_interface::da::{RelevantBlobs, Time};
use sov_test_utils::{TestUser, TEST_DEFAULT_USER_BALANCE};

use crate::stf_blueprint::operator::operator_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{create_blob, PriorityFeeBips, TxStatus, S};

/// Deliberately defined independently of the STF: the skipped height is a fact of
/// reality (the DA block affected by the RPC-node bug), so these tests must verify the
/// literal value `10645809` and fail if the height hardcoded in `apply_slot` is ever
/// edited, accidentally or otherwise.
const FORCED_EMPTY_DA_HEIGHT: u64 = 10_645_809;

fn setup_runner_and_valid_blob() -> (
    sov_test_utils::runtime::TestRunner<IntegTestRuntime<S>, S>,
    RelevantBlobs<MockBlob>,
) {
    let reward_user = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    let (runner, users, _sequencer_account) = setup(reward_user, 2);

    let admin_account = &users[0];
    let not_admin_account = &users[1];

    let mock_blob = create_blob::<IntegTestRuntime<S>>(
        &[TxStatus::Success],
        PriorityFeeBips::from_percentage(0),
        admin_account,
        not_admin_account,
        runner.config.sequencer_da_address,
    );

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![mock_blob],
    };

    (runner, blobs)
}

#[test]
fn blobs_at_forced_empty_da_height_are_dropped_without_trace() {
    let (mut runner, blobs) = setup_runner_and_valid_blob();

    let (result, _) = runner.execute_at_da_height(blobs, FORCED_EMPTY_DA_HEIGHT);

    assert!(
        result.batch_receipts.is_empty(),
        "No batch may execute in a forced-empty slot"
    );
    assert!(
        result.discarded_blobs.is_empty(),
        "Blobs in a forced-empty slot must vanish entirely, not be reported as discarded"
    );
}

/// Control test pinning the exception to exactly the hardcoded height: the same blob
/// one block later executes normally.
#[test]
fn blobs_at_adjacent_da_height_are_executed() {
    let (mut runner, blobs) = setup_runner_and_valid_blob();

    let (result, _) = runner.execute_at_da_height(blobs, FORCED_EMPTY_DA_HEIGHT + 1);

    assert_eq!(
        result.batch_receipts.len(),
        1,
        "A valid blob outside the forced-empty height must execute"
    );
}

/// The forced-empty slot must produce exactly the state transition of a slot whose DA
/// block contains no rollup blobs.
#[test]
fn forced_empty_slot_state_matches_empty_slot_state() {
    let (mut runner, blobs) = setup_runner_and_valid_blob();
    // Freeze time so both simulated headers are identical.
    runner.config.freeze_time = Some(Time::from_millis(1_700_000_000_000));

    let (slot_with_blobs, _, _) = runner.simulate_at_da_height(blobs, FORCED_EMPTY_DA_HEIGHT);
    let empty_blobs = RelevantBlobs::<MockBlob> {
        proof_blobs: vec![],
        batch_blobs: vec![],
    };
    let (empty_slot, _, _) = runner.simulate_at_da_height(empty_blobs, FORCED_EMPTY_DA_HEIGHT);

    assert_eq!(
        slot_with_blobs.state_root.as_ref(),
        empty_slot.state_root.as_ref(),
        "A forced-empty slot with blobs must reach the same state root as an empty slot"
    );
}

//! Tests for the hardcoded consensus exception in `apply_slot` that treats Celestia DA
//! blocks 10645809-10645810 and 10897147-10897150 as containing no rollup blobs.

use sov_mock_da::MockBlob;
use sov_rollup_interface::da::{RelevantBlobs, Time};
use sov_test_utils::{TestUser, TEST_DEFAULT_USER_BALANCE};

use crate::stf_blueprint::operator::operator_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{create_blob, PriorityFeeBips, TxStatus, S};

/// Deliberately defined independently of the STF: the skipped heights are facts of
/// reality (the DA blocks affected by the RPC-node bug), so these tests must verify the
/// literal values `10645809-10645810` and `10897147-10897150`, and fail if the heights
/// hardcoded in `apply_slot` are ever edited, accidentally or otherwise.
const FORCED_EMPTY_DA_HEIGHTS: [u64; 6] = [
    10_645_809, 10_645_810, 10_897_147, 10_897_148, 10_897_149, 10_897_150,
];

/// The DA heights directly surrounding the forced-empty ranges, where execution must
/// behave normally. 10897151 is load-bearing: its blob WAS served to the original node
/// (and buffered out-of-order in kernel state), so it must not be skipped.
const ADJACENT_DA_HEIGHTS: [u64; 4] = [10_645_808, 10_645_811, 10_897_146, 10_897_151];

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
fn blobs_at_forced_empty_da_heights_are_dropped_without_trace() {
    for da_height in FORCED_EMPTY_DA_HEIGHTS {
        let (mut runner, blobs) = setup_runner_and_valid_blob();

        let (result, _) = runner.execute_at_da_height(blobs, da_height);

        assert!(
            result.batch_receipts.is_empty(),
            "No batch may execute in the forced-empty slot at DA height {da_height}"
        );
        assert!(
            result.discarded_blobs.is_empty(),
            "Blobs in the forced-empty slot at DA height {da_height} must vanish entirely, \
             not be reported as discarded"
        );
    }
}

/// Control test pinning the exception to exactly the hardcoded heights: the same blob
/// immediately before and after the forced-empty range executes normally.
#[test]
fn blobs_at_adjacent_da_heights_are_executed() {
    for da_height in ADJACENT_DA_HEIGHTS {
        let (mut runner, blobs) = setup_runner_and_valid_blob();

        let (result, _) = runner.execute_at_da_height(blobs, da_height);

        assert_eq!(
            result.batch_receipts.len(),
            1,
            "A valid blob at DA height {da_height}, outside the forced-empty range, \
             must execute"
        );
    }
}

/// A forced-empty slot must produce exactly the state transition of a slot whose DA
/// block contains no rollup blobs.
#[test]
fn forced_empty_slot_state_matches_empty_slot_state() {
    for da_height in FORCED_EMPTY_DA_HEIGHTS {
        let (mut runner, blobs) = setup_runner_and_valid_blob();
        // Freeze time so both simulated headers are identical.
        runner.config.freeze_time = Some(Time::from_millis(1_700_000_000_000));

        let (slot_with_blobs, _, _) = runner.simulate_at_da_height(blobs, da_height);
        let empty_blobs = RelevantBlobs::<MockBlob> {
            proof_blobs: vec![],
            batch_blobs: vec![],
        };
        let (empty_slot, _, _) = runner.simulate_at_da_height(empty_blobs, da_height);

        assert_eq!(
            slot_with_blobs.state_root.as_ref(),
            empty_slot.state_root.as_ref(),
            "The forced-empty slot at DA height {da_height} with blobs must reach the \
             same state root as an empty slot"
        );
    }
}

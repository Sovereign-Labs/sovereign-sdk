use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sov_mock_da::MockBlock;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{InvalidProofError, ProofOutcome};
use sov_rollup_interface::common::{IntoSlotNumber, SlotNumber};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::optimistic::Attestation;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    assert_matches, ProofInput, ProofTestCase, TestAttester, TEST_ROLLUP_FINALITY_PERIOD,
};

use crate::helpers::{build_proof, make_attestation_blob, setup, TestAttesterIncentives, RT, S};

/// Sets up the invariant tests by executing empty slots and attesting up to `FINALITY_PERIOD + 1`. The maximum attested height is
/// equal to the `FINALITY_PERIOD + 1` at the end of the setup. Returns the test runner, genesis attester and the maximum attested height.
fn setup_invariant_tests() -> (TestRunner<RT, S>, TestAttester<S>, u64) {
    let (mut runner, genesis_attester, _, _) = setup();

    runner.advance_slots(TEST_ROLLUP_FINALITY_PERIOD as usize);

    let expected_max_attested_height =
        SlotNumber::new(TEST_ROLLUP_FINALITY_PERIOD.saturating_add(1));
    // Use atomic, so it can be properly shared with TestRunner closures.
    let max_attested_height_ref = Arc::new(AtomicU64::default());

    // Increase the max attested height by attesting to up to the finality period + 1.
    for height_to_attest in 1i32
        .to_slot_number()
        .range_inclusive(expected_max_attested_height)
    {
        let max_attested_height_ref_loop = max_attested_height_ref.clone();

        let genesis_attester = genesis_attester.clone();
        let attestation_proof = runner
            .query_visible_state(|state| {
                build_proof(
                    state,
                    height_to_attest.get(),
                    &genesis_attester.user_info.address(),
                )
            })
            .unwrap();

        runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
            input: ProofInput(make_attestation_blob(attestation_proof)),
            assert: Box::new(move |result, state| {
                assert_matches!(
                    result.proof_receipt.unwrap().outcome,
                    ProofOutcome::Valid { .. }
                );
                max_attested_height_ref_loop.fetch_add(1, Ordering::SeqCst);

                assert_eq!(
                    TestAttesterIncentives::default()
                        .bonded_attesters
                        .get(&genesis_attester.user_info.address(), state)
                        .unwrap(),
                    Some(genesis_attester.bond),
                    "Bonded amount should not have changed"
                );

                let max_attested_height = TestAttesterIncentives::default()
                    .maximum_attested_height
                    .get(state)
                    .unwrap_infallible()
                    .unwrap();
                assert_eq!(
                    max_attested_height.get(),
                    max_attested_height_ref_loop.load(Ordering::SeqCst),
                    "The max attested height should have increased by 1. Slot height {height_to_attest}"
                );
            }),
        });
    }

    assert_eq!(
        max_attested_height_ref.load(Ordering::SeqCst),
        expected_max_attested_height.get(),
        "Problem in setup"
    );
    (runner, genesis_attester, expected_max_attested_height.get())
}

/// The attesters need to publish attestations for slots above `MAX_ATTESTED_HEIGHT - ROLLUP_FINALITY_PERIOD`.

#[test]
fn test_cannot_attest_below_max_attested_height() {
    let (mut runner, genesis_attester, expected_max_attested_height) = setup_invariant_tests();

    let attestation_proof = runner
        .query_visible_state(|state| build_proof(state, 1, &genesis_attester.user_info.address()))
        .unwrap();

    // Now try to attest to a block at height 1. This is stricly below `MAX_ATTESTED_HEIGHT - TEST_ROLLUP_FINALITY_PERIOD`.
    // The transaction should revert.
    runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
        input: ProofInput(make_attestation_blob(attestation_proof)),
        assert: Box::new(move |result, state| {
            match &result.proof_receipt.unwrap().outcome {
                ProofOutcome::Invalid(e@InvalidProofError::PreconditionNotMet(msg)) => {
                    assert!(!e.is_not_revertable());
                    assert_eq!(msg, "Transition invariant isn't respected");
                }
                _ => panic!("Expected invalid outcome"),
            }

            let max_attested_height = TestAttesterIncentives::default()
                .maximum_attested_height
                .get(state)
                .unwrap_infallible()
                .unwrap();
            let finality = TestAttesterIncentives::default()
                .rollup_finality_period
                .get(state)
                .unwrap_infallible()
                .unwrap();

            assert_eq!(max_attested_height.get(), expected_max_attested_height, "Sanity check failed: the max attested height should be {expected_max_attested_height}, but it is {max_attested_height}");

            assert!(
                max_attested_height > finality,
                "The difference between the max attested height (value: {max_attested_height})
                 and the finality period (value: {finality}) should be greater than 1"
            );
        }),
    });
}

/// The attesters need to publish attestations for slots below `MAX_ATTESTED_HEIGHT + 1`.

#[test]
fn test_cannot_attest_above_max_attested_height_plus_one() {
    let (mut runner, genesis_attester, expected_max_attested_height) = setup_invariant_tests();

    let attestation_proof = runner
        .query_visible_state(|state| build_proof(state, 1, &genesis_attester.user_info.address()))
        .unwrap();

    runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
        input: ProofInput(make_attestation_blob(attestation_proof)),
        assert: Box::new(move |result, state| {

        match &result.proof_receipt.unwrap().outcome {
            ProofOutcome::Invalid(e@InvalidProofError::PreconditionNotMet(msg)) => {
                assert!(!e.is_not_revertable());
                assert_eq!(msg, "Transition invariant isn't respected");
            }
            _ => panic!("Expected invalid outcome"),
        }

        // Ensure that the `MAX_ATTESTED_HEIGHT` increases by 1.
        let max_attested_height = TestAttesterIncentives::default()
            .maximum_attested_height
            .get(state)
            .unwrap_infallible()
            .unwrap();
        assert_eq!(max_attested_height.get(), expected_max_attested_height, "Sanity check failed: the max attested height should be {expected_max_attested_height}, but it is {max_attested_height}");
        }),
    });
}

/// Test that the attesters can publish attestations for slots within the range `MAX_ATTESTED_HEIGHT - ROLLUP_FINALITY_PERIOD` to `MAX_ATTESTED_HEIGHT + 1`.
/// If attesters publish attestations in the range `MAX_ATTESTED_HEIGHT - ROLLUP_FINALITY_PERIOD + 1` to `MAX_ATTESTED_HEIGHT`, the attestations are valid but the max attested height is not updated.
#[test]
fn test_can_attest_within_allowed_range() {
    let (mut runner, genesis_attester, max_attested_height) = setup_invariant_tests();
    // Now try to attest every non-finalized slot again,
    // which are between `MAX_ATTESTED_HEIGHT - FINALITY_PERIOD + 1` and `MAX_ATTESTED_HEIGHT`.
    // Check that the attestations are valid but the sequence is not rewarded.
    let start_height_to_attest = max_attested_height
        .checked_sub(TEST_ROLLUP_FINALITY_PERIOD)
        .expect("Test setup has changed, should have go beyond finalization")
        .checked_add(1)
        .expect("Test setup has changed, rollup should have non-zero finalization period");

    for rollup_height_to_attest in start_height_to_attest..max_attested_height {
        let attestation_proof = runner
            .query_visible_state(|state| {
                build_proof(
                    state,
                    rollup_height_to_attest,
                    &genesis_attester.user_info.address(),
                )
            })
            .unwrap();
        runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
            input: ProofInput(make_attestation_blob(attestation_proof)),
            assert: Box::new(move |result, state| {
                assert_matches!(
                    result.proof_receipt.unwrap().outcome,
                    ProofOutcome::Valid { .. }
                );

                // Ensure that the `MAX_ATTESTED_HEIGHT` does not increase.
                let current_max_attested_height = TestAttesterIncentives::default()
                    .maximum_attested_height
                    .get(state)
                    .unwrap_infallible()
                    .unwrap();
                assert_eq!(
                    max_attested_height, current_max_attested_height.get(),
                    "The max attested height should not have changed. Slot height {rollup_height_to_attest}"
                );
            }),
        });
    }
}

#[test]
fn test_cannot_attest_genesis_height() {
    let (mut runner, genesis_attester, _, _) = setup();

    runner.advance_slots(1);
    // Building genesis attestation

    let attestation_proof = runner.query_visible_state(|state| {
        let genesis_height = RollupHeight::GENESIS;
        let chain_state = sov_chain_state::ChainState::<S>::default();
        let genesis_root_hash = chain_state.get_genesis_hash(state).unwrap().unwrap();

        let mut archival_state = state.get_archival_state(genesis_height).unwrap();

        let proof_of_bond = TestAttesterIncentives::default()
            .bonded_attesters
            .get_with_proof(&genesis_attester.user_info.address(), &mut archival_state)
            .unwrap();

        let genesis_block_hash = MockBlock::default().header.hash();

        Attestation {
            initial_state_root: genesis_root_hash,
            slot_hash: genesis_block_hash,
            post_state_root: genesis_root_hash,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_slot_number: SlotNumber::GENESIS,
                proof: proof_of_bond,
            },
        }
    });

    runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
        input: ProofInput(make_attestation_blob(attestation_proof)),
        assert: Box::new(
            move |result, _state| match &result.proof_receipt.unwrap().outcome {
                ProofOutcome::Invalid(e @ InvalidProofError::PreconditionNotMet(msg)) => {
                    assert!(!e.is_not_revertable());
                    assert_eq!(msg, "Transition invariant isn't respected");
                }
                outcome => {
                    panic!("Unexpected proof outcome {outcome:?}, expected ProofOutcome::Invalid")
                }
            },
        ),
    });
}

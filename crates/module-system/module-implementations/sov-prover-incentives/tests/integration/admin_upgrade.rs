//! Tests for the admin bypass + upgrade flow in [`ProverIncentives`].

use sov_modules_api::{
    AggregatedProofPublicData, CodeCommitmentHash, CodeCommitmentTrait, ProofOutcome, Spec,
};
use sov_rollup_interface::common::IntoSlotNumber;
use sov_state::Storage;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{assert_matches, ProofInput, ProofTestCase, TestProver, TestSpec};

use crate::helpers::{
    build_proof, consume_gas_tx_for_signer, serialize_proof, serialize_proof_with_commitment,
    setup, TestProverIncentives, RT,
};

type S = TestSpec;

#[allow(clippy::type_complexity)]
fn prepare_admin_proof() -> (
    TestRunner<RT, S>,
    TestProver<S>,
    AggregatedProofPublicData<
        <S as Spec>::Address,
        <S as Spec>::Da,
        <<S as Spec>::Storage as Storage>::Root,
    >,
) {
    let (mut runner, prover, other_user) = setup();

    for _ in 0..3 {
        runner.execute(consume_gas_tx_for_signer(&other_user));
    }

    let aggregated_proof = runner
        .query_visible_state(|state| {
            build_proof(
                state,
                1.to_slot_number(),
                2.to_slot_number(),
                prover.user_info.address(),
            )
        })
        .unwrap();

    (runner, prover, aggregated_proof)
}

/// Admin proofs with a new `origin_state_root` are NOT slashed for
/// `IncorrectGenesisHash` — instead, the new value is recorded as an admin
/// upgrade keyed by `final_slot_number`, and the pointer advances.
#[test]
fn test_admin_bypass_genesis_hash_records_upgrade() {
    let (mut runner, _prover, mut aggregated_proof) = prepare_admin_proof();
    // Force origin_state_root to differ from chain_state's genesis hash.
    aggregated_proof
        .origin_state_root
        .clone_from(&aggregated_proof.final_state_root);

    let expected_slot = aggregated_proof.final_slot_number;
    let expected_origin = aggregated_proof.origin_state_root;

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );

            let module = TestProverIncentives::default();
            assert_eq!(
                module.latest_admin_upgrade_slot.get(state).unwrap(),
                Some(expected_slot),
                "latest_admin_upgrade_slot should advance to the proof's final slot",
            );
            let upgrade = module
                .admin_upgrades
                .get(&expected_slot, state)
                .unwrap()
                .expect("upgrade entry should be recorded at the proof's final slot");
            assert_eq!(upgrade.origin_state_root, expected_origin);
        }),
    });
}

/// Admin proofs with a new `outer_vk_hash` are NOT slashed for verification
/// failure — the outer commitment is reconstructed from the claimed hash, the
/// proof is verified against it, and the new commitment is recorded. This is
/// the most consequential rotation path: it changes which proofs count as
/// valid from this slot onward.
#[test]
fn test_admin_bypass_outer_vk_hash_records_upgrade() {
    let (mut runner, _prover, mut aggregated_proof) = prepare_admin_proof();
    // MockCodeCommitment is 8 bytes wide and zero-pads to 32 in `to_hash`, so we use a
    // value that round-trips losslessly under from_hash/to_hash.
    let new_outer_commitment = sov_mock_zkvm::MockCodeCommitment([0xcd; 8]);
    let new_outer_vk_hash = {
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(&[0xcd; 8]);
        CodeCommitmentHash(bytes)
    };
    aggregated_proof.outer_vk_hash = new_outer_vk_hash.clone();
    let expected_slot = aggregated_proof.final_slot_number;

    // Stamp the new commitment into the proof bytes so the admin-path verifier
    // (which rebuilds its commitment from `outer_vk_hash`) accepts the proof.
    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof_with_commitment(
            aggregated_proof,
            new_outer_commitment,
        )),
        assert: Box::new(move |result, state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );

            let module = TestProverIncentives::default();
            assert_eq!(
                module.latest_admin_upgrade_slot.get(state).unwrap(),
                Some(expected_slot),
                "latest_admin_upgrade_slot should advance to the proof's final slot",
            );
            let upgrade = module
                .admin_upgrades
                .get(&expected_slot, state)
                .unwrap()
                .expect("upgrade entry should be recorded at the proof's final slot");
            assert_eq!(upgrade.outer_code_commitment.to_hash(), new_outer_vk_hash);
        }),
    });
}

/// Admin proofs with a new `inner_vkey_hash` are NOT slashed for
/// `IncorrectInnerVkeyHash` — instead, the new inner commitment is recorded.
#[test]
fn test_admin_bypass_inner_vkey_hash_records_upgrade() {
    let (mut runner, _prover, mut aggregated_proof) = prepare_admin_proof();
    // MockCodeCommitment is 8 bytes wide and zero-pads to 32 in `to_hash`, so we use a
    // value that round-trips losslessly under from_hash/to_hash.
    let new_inner_vkey_hash = {
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(&[0xab; 8]);
        CodeCommitmentHash(bytes)
    };
    aggregated_proof.inner_vkey_hash = new_inner_vkey_hash.clone();
    let expected_slot = aggregated_proof.final_slot_number;

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );

            let module = TestProverIncentives::default();
            let upgrade = module
                .admin_upgrades
                .get(&expected_slot, state)
                .unwrap()
                .expect("upgrade entry should be recorded");
            assert_eq!(upgrade.inner_code_commitment.to_hash(), new_inner_vkey_hash,);
        }),
    });
}

/// An admin proof whose public outputs already match the current effective
/// values is a normal proof, not an upgrade — no map entry should be added
/// and the latest-pointer should remain unset.
#[test]
fn test_admin_matching_proof_does_not_record_upgrade() {
    let (mut runner, _prover, aggregated_proof) = prepare_admin_proof();
    let final_slot = aggregated_proof.final_slot_number;

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );

            let module = TestProverIncentives::default();
            assert_eq!(
                module.latest_admin_upgrade_slot.get(state).unwrap(),
                None,
                "no upgrade should be recorded for a matching admin proof",
            );
            assert!(module
                .admin_upgrades
                .get(&final_slot, state)
                .unwrap()
                .is_none());
        }),
    });
}

/// An admin proof that *would* be an upgrade (its public outputs differ from
/// current effective values) but whose slot range has already been claimed
/// (Penalized) must NOT mutate the upgrade history: persisting the upgrade
/// while the proof itself is rejected would desync chain state from off-chain
/// prover state across restarts.
#[test]
fn test_admin_upgrade_not_recorded_on_penalized_proof() {
    let (mut runner, prover, mut first_proof) = prepare_admin_proof();
    // First proof for slots 1->2: a normal (no-op) admin proof that claims the
    // reward and pins last_claimed_reward to slot 2.
    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(first_proof.clone())),
        assert: Box::new(move |result, _state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );
        }),
    });

    // Rebuild the same range with a mutated origin_state_root so the admin
    // proof looks like an upgrade attempt - but for an already-claimed range,
    // so `calculate_reward_and_remove` should return Penalized.
    first_proof
        .origin_state_root
        .clone_from(&first_proof.final_state_root);
    let prover_address = prover.user_info.address();
    let final_slot = first_proof.final_slot_number;

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(first_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                &result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(e, _) if matches!(
                    e,
                    sov_modules_api::InvalidProofError::ProverPenalized(_)
                )
            );

            let module = TestProverIncentives::default();
            assert_eq!(
                module.latest_admin_upgrade_slot.get(state).unwrap(),
                None,
                "Penalized admin proof must not advance latest_admin_upgrade_slot",
            );
            assert!(
                module
                    .admin_upgrades
                    .get(&final_slot, state)
                    .unwrap()
                    .is_none(),
                "Penalized admin proof must not write an admin_upgrades entry",
            );
            // The admin is penalized (loses bond), not slashed; they remain
            // bonded after the deduction.
            assert!(module
                .bonded_provers
                .get(&prover_address, state)
                .unwrap()
                .is_some());
        }),
    });
}

/// Once an admin upgrade is accepted, any later-submitted proof whose claimed
/// range starts before the upgrade slot is stale and must be rejected without
/// slashing, even if its final slot is newer than the cutoff.
#[test]
fn wh() {
    let (mut runner, prover, mut first_proof) = prepare_admin_proof();
    first_proof
        .origin_state_root
        .clone_from(&first_proof.final_state_root);
    let upgrade_slot = first_proof.final_slot_number;
    let first_origin = first_proof.origin_state_root;

    // First admin upgrade succeeds and pins `latest_admin_upgrade_slot` to slot 2.
    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(first_proof)),
        assert: Box::new(move |result, _state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );
        }),
    });

    for _ in 0..2 {
        runner.execute(consume_gas_tx_for_signer(&prover.user_info));
    }

    // Build a proof that crosses the cutoff. It is syntactically valid but its
    // claimed range still begins before the accepted upgrade slot, so it should
    // be rejected as stale before any slashing path runs.
    let stale_proof = runner
        .query_visible_state(|state| {
            build_proof(
                state,
                1.to_slot_number(),
                4.to_slot_number(),
                prover.user_info.address(),
            )
        })
        .unwrap();
    let prover_address = prover.user_info.address();
    let stale_final_slot = stale_proof.final_slot_number;
    let expected_reason = format!(
        "Proof range [1, {}] starts before latest admin upgrade slot {}",
        stale_final_slot, upgrade_slot
    );

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(stale_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                &result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(e, _) if matches!(
                    e,
                    sov_modules_api::InvalidProofError::PreconditionNotMet(reason)
                        if reason == &expected_reason
                )
            );

            let module = TestProverIncentives::default();
            assert!(
                module
                    .bonded_provers
                    .get(&prover_address, state)
                    .unwrap()
                    .is_some(),
                "stale pre-upgrade proof must not slash the prover",
            );
            assert_eq!(
                module.latest_admin_upgrade_slot.get(state).unwrap(),
                Some(upgrade_slot),
            );
            let recorded = module
                .admin_upgrades
                .get(&upgrade_slot, state)
                .unwrap()
                .unwrap();
            assert_eq!(recorded.origin_state_root, first_origin);
            assert!(
                module
                    .admin_upgrades
                    .get(&stale_final_slot, state)
                    .unwrap()
                    .is_none(),
                "stale proof must not create a newer upgrade entry",
            );
        }),
    });
}

/// An admin proof that *would* be an upgrade but targets a `final_slot_number`
/// that is not strictly newer than the most recent recorded upgrade must be
/// slashed for `StaleAdminUpgrade`. Accepting it would rewind the canonical
/// commitments to an older slot, so this path is security-critical.
#[test]
fn test_admin_upgrade_at_same_slot_is_slashed() {
    let (mut runner, prover, mut first_proof) = prepare_admin_proof();
    first_proof
        .origin_state_root
        .clone_from(&first_proof.final_state_root);
    let upgrade_slot = first_proof.final_slot_number;
    let first_origin = first_proof.origin_state_root;

    // First admin upgrade pins `latest_admin_upgrade_slot` to `upgrade_slot`.
    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(first_proof)),
        assert: Box::new(move |result, _state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );
        }),
    });

    // Build a second admin proof for [upgrade_slot, upgrade_slot]. Its initial
    // slot satisfies the stale-proof cutoff (>= latest_admin_upgrade_slot), but
    // its final slot is not strictly newer than the recorded upgrade. Mutating
    // `outer_vk_hash` makes it look like an upgrade attempt so the stale-slot
    // check fires.
    let mut second_proof = runner
        .query_visible_state(|state| {
            build_proof(
                state,
                upgrade_slot,
                upgrade_slot,
                prover.user_info.address(),
            )
        })
        .unwrap();
    let new_outer_commitment = sov_mock_zkvm::MockCodeCommitment([0xef; 8]);
    second_proof.outer_vk_hash = {
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(&[0xef; 8]);
        CodeCommitmentHash(bytes)
    };
    let prover_address = prover.user_info.address();

    // Stamp the mutated commitment into the proof bytes so the admin-path
    // verifier (which rebuilds its commitment from `outer_vk_hash`) accepts
    // the proof and execution reaches the stale-upgrade check.
    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof_with_commitment(
            second_proof,
            new_outer_commitment,
        )),
        assert: Box::new(move |result, state| {
            assert_matches!(
                &result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(e, _) if matches!(
                    e,
                    sov_modules_api::InvalidProofError::ProverSlashed(reason)
                        if reason == "Invalid output StaleAdminUpgrade"
                )
            );

            let module = TestProverIncentives::default();
            assert!(
                module
                    .bonded_provers
                    .get(&prover_address, state)
                    .unwrap()
                    .is_none(),
                "admin must be slashed on a stale upgrade attempt",
            );
            assert_eq!(
                module.latest_admin_upgrade_slot.get(state).unwrap(),
                Some(upgrade_slot),
                "stale upgrade must not advance latest_admin_upgrade_slot",
            );
            let recorded = module
                .admin_upgrades
                .get(&upgrade_slot, state)
                .unwrap()
                .expect("first upgrade should still be recorded");
            assert_eq!(
                recorded.origin_state_root, first_origin,
                "first upgrade's recorded origin must not be overwritten",
            );
        }),
    });
}

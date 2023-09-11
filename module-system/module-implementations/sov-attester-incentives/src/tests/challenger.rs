use borsh::BorshSerialize;
use sov_modules_api::default_context::DefaultContext;
use sov_rollup_interface::mocks::{
    MockCodeCommitment, MockDaSpec, MockProof, MockValidityCond, MockValidityCondChecker,
};
use sov_rollup_interface::zk::StateTransition;
use sov_state::{ProverStorage, WorkingSet};

use crate::call::{AttesterIncentiveErrors, SlashingReason};
use crate::tests::helpers::{
    commit_get_new_working_set, execution_simulation, setup, BOND_AMOUNT, INITIAL_BOND_AMOUNT,
    INIT_HEIGHT,
};

/// Test that given an invalid transition, a challenger can successfully challenge it and get rewarded
#[test]
fn test_valid_challenge() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, token_address, attester_address, challenger_address) = setup(&mut working_set);

    let working_set = commit_get_new_working_set(&storage, working_set);

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (mut exec_vars, mut working_set) =
        execution_simulation(3, &module, &storage, attester_address, working_set);

    let _ = exec_vars.pop().unwrap();
    let transition_1 = exec_vars.pop().unwrap();
    let initial_transition = exec_vars.pop().unwrap();

    module
        .bond_user_helper(
            BOND_AMOUNT,
            &challenger_address,
            crate::call::Role::Challenger,
            &mut working_set,
        )
        .unwrap();

    // Assert that the challenger has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                challenger_address,
                crate::call::Role::Challenger,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Set a bad transition to get a reward from
    module
        .bad_transition_pool
        .set(&(INIT_HEIGHT + 1), &BOND_AMOUNT, &mut working_set);

    // Process a correct challenge
    let context = DefaultContext {
        sender: challenger_address,
    };

    {
        let transition = StateTransition::<MockDaSpec, _> {
            initial_state_root: initial_transition.state_root,
            slot_hash: [1; 32].into(),
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: MockValidityCond { is_valid: true },
        };

        let serialized_transition = transition.try_to_vec().unwrap();

        let commitment = module
            .commitment_to_allowed_challenge_method
            .get(&mut working_set)
            .expect("Should be set at genesis");

        let proof = &MockProof {
            program_id: commitment,
            is_valid: true,
            log: serialized_transition.as_slice(),
        }
        .encode_to_vec();

        module
            .process_challenge(
                &context,
                proof.as_slice(),
                &(INIT_HEIGHT + 1),
                &mut working_set,
            )
            .expect("Should not fail");

        // Check that the challenger was rewarded
        assert_eq!(
            module
                .bank
                .get_balance_of(challenger_address, token_address, &mut working_set)
                .unwrap(),
            INITIAL_BOND_AMOUNT - BOND_AMOUNT + BOND_AMOUNT / 2,
            "The challenger should have been rewarded"
        );

        // Check that the challenge set is empty
        assert_eq!(
            module
                .bad_transition_pool
                .get(&(INIT_HEIGHT + 1), &mut working_set),
            None,
            "The transition should have disappeared"
        )
    }

    {
        // Now try to unbond the challenger
        module
            .unbond_challenger(&context, &mut working_set)
            .expect("The challenger should be able to unbond");

        // Check the final balance of the challenger
        assert_eq!(
            module
                .bank
                .get_balance_of(challenger_address, token_address, &mut working_set)
                .unwrap(),
            INITIAL_BOND_AMOUNT + BOND_AMOUNT / 2,
            "The challenger should have been unbonded"
        )
    }
}

fn invalid_proof_helper(
    context: &DefaultContext,
    proof: &Vec<u8>,
    reason: SlashingReason,
    challenger_address: sov_modules_api::Address,
    module: &crate::AttesterIncentives<
        DefaultContext,
        sov_rollup_interface::mocks::MockZkvm,
        MockDaSpec,
        MockValidityCondChecker<MockValidityCond>,
    >,
    working_set: &mut WorkingSet<ProverStorage<sov_state::DefaultStorageSpec>>,
) {
    // Let's bond the challenger and try to publish a false challenge
    module
        .bond_user_helper(
            BOND_AMOUNT,
            &challenger_address,
            crate::call::Role::Challenger,
            working_set,
        )
        .expect("Should be able to bond");

    let err = module
        .process_challenge(context, proof.as_slice(), &(INIT_HEIGHT + 1), working_set)
        .unwrap_err();

    // Check the error raised
    assert_eq!(
        err,
        AttesterIncentiveErrors::UserSlashed(reason),
        "The challenge processing should fail with an invalid proof error"
    )
}

#[test]
fn test_invalid_challenge() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, _token_address, attester_address, challenger_address) = setup(&mut working_set);

    let working_set = commit_get_new_working_set(&storage, working_set);

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (mut exec_vars, mut working_set) =
        execution_simulation(3, &module, &storage, attester_address, working_set);

    let _ = exec_vars.pop().unwrap();
    let transition_1 = exec_vars.pop().unwrap();
    let initial_transition = exec_vars.pop().unwrap();

    // Set a bad transition to get a reward from
    module
        .bad_transition_pool
        .set(&(INIT_HEIGHT + 1), &BOND_AMOUNT, &mut working_set);

    // Process a correct challenge but without a bonded attester
    let context = DefaultContext {
        sender: challenger_address,
    };

    let transition: StateTransition<MockDaSpec, _> = StateTransition {
        initial_state_root: initial_transition.state_root,
        slot_hash: [1; 32].into(),
        final_state_root: transition_1.state_root,
        rewarded_address: challenger_address,
        validity_condition: MockValidityCond { is_valid: true },
    };

    let serialized_transition = transition.try_to_vec().unwrap();

    let commitment = module
        .commitment_to_allowed_challenge_method
        .get(&mut working_set)
        .expect("Should be set at genesis");

    {
        // A valid proof
        let proof = &MockProof {
            program_id: commitment.clone(),
            is_valid: true,
            log: serialized_transition.as_slice(),
        }
        .encode_to_vec();

        let err = module
            .process_challenge(
                &context,
                proof.as_slice(),
                &(INIT_HEIGHT + 1),
                &mut working_set,
            )
            .unwrap_err();

        // Check the error raised
        assert_eq!(
            err,
            AttesterIncentiveErrors::UserNotBonded,
            "The challenge processing should fail with an unbonded error"
        )
    }

    // Invalid proofs
    {
        // An invalid proof
        let proof = &MockProof {
            program_id: commitment.clone(),
            is_valid: false,
            log: serialized_transition.as_slice(),
        }
        .encode_to_vec();

        invalid_proof_helper(
            &context,
            proof,
            SlashingReason::InvalidProofOutputs,
            challenger_address,
            &module,
            &mut working_set,
        );

        // Bad slot hash
        let bad_transition = StateTransition::<MockDaSpec, _> {
            initial_state_root: initial_transition.state_root,
            slot_hash: [2; 32].into(),
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: MockValidityCond { is_valid: true },
        }
        .try_to_vec()
        .unwrap();

        // An invalid proof
        let proof = &MockProof {
            program_id: commitment,
            is_valid: true,
            log: bad_transition.as_slice(),
        }
        .encode_to_vec();

        invalid_proof_helper(
            &context,
            proof,
            SlashingReason::TransitionInvalid,
            challenger_address,
            &module,
            &mut working_set,
        );

        // Bad validity condition
        let bad_transition = StateTransition::<MockDaSpec, _> {
            initial_state_root: initial_transition.state_root,
            slot_hash: [1; 32].into(),
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: MockValidityCond { is_valid: false },
        }
        .try_to_vec()
        .unwrap();

        // An invalid proof
        let proof = &MockProof {
            program_id: MockCodeCommitment([0; 32]),
            is_valid: true,
            log: bad_transition.as_slice(),
        }
        .encode_to_vec();

        invalid_proof_helper(
            &context,
            proof,
            SlashingReason::TransitionInvalid,
            challenger_address,
            &module,
            &mut working_set,
        );

        // Bad initial root
        let bad_transition = StateTransition::<MockDaSpec, _> {
            initial_state_root: transition_1.state_root,
            slot_hash: [1; 32].into(),
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: MockValidityCond { is_valid: true },
        }
        .try_to_vec()
        .unwrap();

        // An invalid proof
        let proof = &MockProof {
            program_id: MockCodeCommitment([0; 32]),
            is_valid: true,
            log: bad_transition.as_slice(),
        }
        .encode_to_vec();

        invalid_proof_helper(
            &context,
            proof,
            SlashingReason::InvalidInitialHash,
            challenger_address,
            &module,
            &mut working_set,
        );
    }
}

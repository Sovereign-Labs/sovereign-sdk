use borsh::BorshSerialize;
use sov_modules_api::default_context::DefaultContext;
use sov_rollup_interface::mocks::{
    MockCodeCommitment, MockProof, TestValidityCond, TestValidityCondChecker,
};
use sov_rollup_interface::optimistic::Attestation;
use sov_rollup_interface::zk::StateTransition;
use sov_state::{ProverStorage, WorkingSet};

use crate::call::{AttesterIncentiveErrors, SlashingReason};
use crate::helpers::{
    commit_get_new_working_set, execution_simulation, setup, BOND_AMOUNT, INITIAL_BOND_AMOUNT,
    INIT_HEIGHT,
};

/// Start by testing the positive case where the attestations are valid
#[test]
fn test_process_valid_attestation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, token_address, attester_address, _) = setup(&mut working_set);

    // Assert that the attester has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (mut exec_vars, mut working_set) =
        execution_simulation(3, &module, &storage, attester_address, working_set);

    let context = DefaultContext {
        sender: attester_address,
    };

    let transition_2 = exec_vars.pop().unwrap();
    let transition_1 = exec_vars.pop().unwrap();
    let initial_transition = exec_vars.pop().unwrap();

    // Process a valid attestation for the first transition
    {
        let attestation = Attestation {
            initial_state_root: initial_transition.state_root,
            da_block_hash: [1; 32],
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 1,
                proof: initial_transition.state_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is an error");
    }

    // We can now proceed with the next attestation
    {
        let attestation = Attestation {
            initial_state_root: transition_1.state_root,
            da_block_hash: [2; 32],
            post_state_root: transition_2.state_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 2,
                proof: transition_1.state_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is an error");
    }

    // Assert that the attester's bond amount has not been burned
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Assert that the attester has been awarded the tokens
    assert_eq!(
        module
            .bank
            .get_balance_of(attester_address, token_address, &mut working_set)
            .unwrap(),
        // The attester is bonded at the beginning so he loses BOND_AMOUNT
        INITIAL_BOND_AMOUNT - BOND_AMOUNT + 2 * BOND_AMOUNT
    );
}

#[test]
fn test_burn_on_invalid_attestation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, _token_address, attester_address, _) = setup(&mut working_set);

    // Assert that the prover has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (mut exec_vars, mut working_set) =
        execution_simulation(3, &module, &storage, attester_address, working_set);

    let transition_2 = exec_vars.pop().unwrap();
    let transition_1 = exec_vars.pop().unwrap();
    let initial_transition = exec_vars.pop().unwrap();

    let context = DefaultContext {
        sender: attester_address,
    };

    // Process an invalid proof for genesis: everything is correct except the storage proof.
    // Must simply return an error. Cannot burn the token at this point because we don't know if the
    // sender is bonded or not.
    {
        let attestation = Attestation {
            initial_state_root: initial_transition.state_root,
            da_block_hash: [1; 32],
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 1,
                proof: transition_1.state_proof.clone(),
            },
        };

        let attestation_error = module
            .process_attestation(attestation, &context, &mut working_set)
            .unwrap_err();

        assert_eq!(
            attestation_error,
            AttesterIncentiveErrors::InvalidBondingProof,
            "The bonding proof should fail"
        );
    }

    // Assert that the prover's bond amount has not been burned
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Now proccess a valid attestation for genesis.
    {
        let attestation = Attestation {
            initial_state_root: initial_transition.state_root,
            da_block_hash: [1; 32],
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 1,
                proof: initial_transition.state_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is an error");
    }

    // Then process a new attestation having the wrong initial state root. The attester must be slashed, and the fees burnt
    {
        let attestation = Attestation {
            initial_state_root: initial_transition.state_root,
            da_block_hash: [2; 32],
            post_state_root: transition_2.state_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 2,
                proof: transition_1.state_proof.clone(),
            },
        };

        let attestation_error = module
            .process_attestation(attestation, &context, &mut working_set)
            .unwrap_err();

        assert_eq!(
            attestation_error,
            AttesterIncentiveErrors::UserSlashed(crate::call::SlashingReason::InvalidInitialHash)
        )
    }

    // Check that the attester's bond has been burnt
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        0
    );

    // Check that the attestation is not part of the challengeable set
    assert!(
        module
            .bad_transition_pool
            .get(&(INIT_HEIGHT + 2), &mut working_set)
            .is_none(),
        "The transition should not exist in the pool"
    );

    // Bond the attester once more
    module
        .bond_user_helper(
            BOND_AMOUNT,
            &attester_address,
            crate::call::Role::Attester,
            &mut working_set,
        )
        .unwrap();

    // Process an attestation that has the right bonding proof and initial hash but has a faulty post transition hash.
    {
        let attestation = Attestation {
            initial_state_root: transition_1.state_root,
            da_block_hash: [2; 32],
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 2,
                proof: transition_1.state_proof,
            },
        };

        let attestation_error = module
            .process_attestation(attestation, &context, &mut working_set)
            .unwrap_err();

        assert_eq!(
            attestation_error,
            AttesterIncentiveErrors::UserSlashed(crate::call::SlashingReason::TransitionInvalid)
        )
    }

    // Check that the attester's bond has been burnt
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        0
    );

    // The attestation should be part of the challengeable set and its associated value should be the BOND_AMOUNT
    assert_eq!(
        module
            .bad_transition_pool
            .get(&(INIT_HEIGHT + 2), &mut working_set)
            .unwrap(),
        BOND_AMOUNT,
        "The transition should not exist in the pool"
    );
}

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
        let transition = StateTransition {
            initial_state_root: initial_transition.state_root,
            slot_hash: [1; 32],
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: TestValidityCond { is_valid: true },
        };

        let serialized_transition = transition.try_to_vec().unwrap();

        let commitment = module
            .commitment_to_allowed_challenge_method
            .get(&mut working_set)
            .expect("Should be set at genesis")
            .commitment;

        let proof = &MockProof {
            program_id: commitment,
            is_valid: true,
            log: serialized_transition.as_slice(),
        }
        .encode_to_vec();

        module
            .process_challenge(
                proof.as_slice(),
                INIT_HEIGHT + 1,
                &context,
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
    proof: &Vec<u8>,
    reason: SlashingReason,
    challenger_address: sov_modules_api::Address,
    context: &DefaultContext,
    module: &crate::AttesterIncentives<
        DefaultContext,
        sov_rollup_interface::mocks::MockZkvm,
        TestValidityCond,
        TestValidityCondChecker<TestValidityCond>,
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
        .process_challenge(proof.as_slice(), INIT_HEIGHT + 1, context, working_set)
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

    let transition = StateTransition {
        initial_state_root: initial_transition.state_root,
        slot_hash: [1; 32],
        final_state_root: transition_1.state_root,
        rewarded_address: challenger_address,
        validity_condition: TestValidityCond { is_valid: true },
    };

    let serialized_transition = transition.try_to_vec().unwrap();

    let commitment = module
        .commitment_to_allowed_challenge_method
        .get(&mut working_set)
        .expect("Should be set at genesis")
        .commitment;

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
                proof.as_slice(),
                INIT_HEIGHT + 1,
                &context,
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
            proof,
            SlashingReason::InvalidProofOutputs,
            challenger_address,
            &context,
            &module,
            &mut working_set,
        );

        // Bad slot hash
        let bad_transition = StateTransition {
            initial_state_root: initial_transition.state_root,
            slot_hash: [2; 32],
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: TestValidityCond { is_valid: true },
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
            proof,
            SlashingReason::TransitionInvalid,
            challenger_address,
            &context,
            &module,
            &mut working_set,
        );

        // Bad validity condition
        let bad_transition = StateTransition {
            initial_state_root: initial_transition.state_root,
            slot_hash: [1; 32],
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: TestValidityCond { is_valid: false },
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
            proof,
            SlashingReason::TransitionInvalid,
            challenger_address,
            &context,
            &module,
            &mut working_set,
        );

        // Bad initial root
        let bad_transition = StateTransition {
            initial_state_root: transition_1.state_root,
            slot_hash: [1; 32],
            final_state_root: transition_1.state_root,
            rewarded_address: challenger_address,
            validity_condition: TestValidityCond { is_valid: true },
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
            proof,
            SlashingReason::InvalidInitialHash,
            challenger_address,
            &context,
            &module,
            &mut working_set,
        );
    }
}

// Test the transition invariant and the two phase unbonding for the attester
#[test]
fn test_unbonding() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, _token_address, attester_address, _) = setup(&mut working_set);

    // Assert that the attester has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    const NEW_LIGHT_CLIENT_HEIGHT: u64 = 5;

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (exec_vars, mut working_set) =
        execution_simulation(20, &module, &storage, attester_address, working_set);

    let context = DefaultContext {
        sender: attester_address,
    };

    // Update the finalized height and try to prove the INIT_HEIGHT: should fail
    module
        .light_client_finalized_height
        .set(&(INIT_HEIGHT + NEW_LIGHT_CLIENT_HEIGHT), &mut working_set);

    // Process a valid attestation for the first transition *should fail*
    {
        let attestation = Attestation {
            initial_state_root: exec_vars[0].state_root,
            da_block_hash: [1; 32],
            post_state_root: exec_vars[1].state_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 1,
                proof: exec_vars[usize::try_from(INIT_HEIGHT).unwrap()]
                    .state_proof
                    .clone(),
            },
        };

        let err = module
            .process_attestation(attestation, &context, &mut working_set)
            .unwrap_err();

        assert_eq!(
            err,
            AttesterIncentiveErrors::InvalidTransitionInvariant,
            "Incorrect error raised"
        );

        // The attester should not be slashed
        assert_eq!(
            module
                .get_bond_amount(
                    attester_address,
                    crate::call::Role::Attester,
                    &mut working_set
                )
                .value,
            BOND_AMOUNT
        );
    }

    // The attester should be able to process multiple attestations with the same bonding proof
    // for i in 1..usize::try_from(DEFAULT_ROLLUP_FINALITY).unwrap() {
    //     println!("{i}");
    //     let new_height = usize::try_from(INIT_HEIGHT + NEW_LIGHT_CLIENT_HEIGHT + 2).unwrap();
    //     let attestation = Attestation {
    //         initial_state_root: exec_vars[new_height + i].state_root.clone(),
    //         da_block_hash: [new_height.try_into().unwrap(); 32],
    //         post_state_root: exec_vars[new_height + i + 1].state_root.clone(),
    //         proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
    //             transition_num: new_height.try_into().unwrap(),
    //             proof: exec_vars[new_height - 1].state_proof.clone(),
    //         },
    //     };

    //     module
    //         .process_attestation(attestation, &context, &mut working_set)
    //         .unwrap();
    // }
}

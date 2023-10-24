use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::optimistic::Attestation;
use sov_modules_api::WorkingSet;
use sov_state::ProverStorage;

use crate::call::AttesterIncentiveErrors;
use crate::tests::helpers::{
    execution_simulation, setup, BOND_AMOUNT, INITIAL_BOND_AMOUNT, INIT_HEIGHT,
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
            da_block_hash: [1; 32].into(),
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 1,
                proof: initial_transition.state_proof,
            },
        }
        .into();

        module
            .process_attestation(&context, attestation, &mut working_set)
            .expect("An invalid proof is an error");
    }

    // We can now proceed with the next attestation
    {
        let attestation = Attestation {
            initial_state_root: transition_1.state_root,
            da_block_hash: [2; 32].into(),
            post_state_root: transition_2.state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 2,
                proof: transition_1.state_proof,
            },
        };

        module
            .process_attestation(&context, attestation.into(), &mut working_set)
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
            da_block_hash: [1; 32].into(),
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 1,
                proof: transition_1.state_proof.clone(),
            },
        };

        let attestation_error = module
            .process_attestation(&context, attestation.into(), &mut working_set)
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
            da_block_hash: [1; 32].into(),
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 1,
                proof: initial_transition.state_proof,
            },
        };

        module
            .process_attestation(&context, attestation.into(), &mut working_set)
            .expect("An invalid proof is an error");
    }

    // Then process a new attestation having the wrong initial state root. The attester must be slashed, and the fees burnt
    {
        let attestation = Attestation {
            initial_state_root: initial_transition.state_root,
            da_block_hash: [2; 32].into(),
            post_state_root: transition_2.state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 2,
                proof: transition_1.state_proof.clone(),
            },
        };

        let attestation_error = module
            .process_attestation(&context, attestation.into(), &mut working_set)
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
            da_block_hash: [2; 32].into(),
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 2,
                proof: transition_1.state_proof,
            },
        };

        let attestation_error = module
            .process_attestation(&context, attestation.into(), &mut working_set)
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

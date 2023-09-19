use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::optimistic::Attestation;
use sov_modules_api::WorkingSet;
use sov_state::ProverStorage;

use crate::call::AttesterIncentiveErrors;
use crate::tests::helpers::{
    execution_simulation, setup, BOND_AMOUNT, DEFAULT_ROLLUP_FINALITY, INIT_HEIGHT,
};

// Test the transition invariant
#[test]
fn test_transition_invariant() {
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

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (exec_vars, mut working_set) =
        execution_simulation(20, &module, &storage, attester_address, working_set);

    let context = DefaultContext {
        sender: attester_address,
    };

    const NEW_LIGHT_CLIENT_FINALIZED_HEIGHT: u64 = DEFAULT_ROLLUP_FINALITY + INIT_HEIGHT + 1;

    // Update the finalized height and try to prove the INIT_HEIGHT: should fail
    module
        .light_client_finalized_height
        .set(&NEW_LIGHT_CLIENT_FINALIZED_HEIGHT, &mut working_set);

    // Update the initial height
    module
        .maximum_attested_height
        .set(&NEW_LIGHT_CLIENT_FINALIZED_HEIGHT, &mut working_set);

    // Process a valid attestation for the first transition *should fail*
    {
        let init_height_usize = usize::try_from(INIT_HEIGHT).unwrap();
        let attestation = Attestation {
            initial_state_root: exec_vars[init_height_usize].state_root,
            da_block_hash: [(init_height_usize + 1).try_into().unwrap(); 32].into(),
            post_state_root: exec_vars[init_height_usize + 1].state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 1,
                proof: exec_vars[init_height_usize].state_proof.clone(),
            },
        };

        let err = module
            .process_attestation(&context, attestation.into(), &mut working_set)
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

    let new_height = usize::try_from(NEW_LIGHT_CLIENT_FINALIZED_HEIGHT).unwrap();

    // The attester should be able to process multiple attestations with the same bonding proof
    for i in 0..usize::try_from(DEFAULT_ROLLUP_FINALITY + 1).unwrap() {
        let old_attestation = Attestation {
            initial_state_root: exec_vars[new_height - 1].state_root,
            da_block_hash: [(new_height).try_into().unwrap(); 32].into(),
            post_state_root: exec_vars[new_height].state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: new_height.try_into().unwrap(),
                proof: exec_vars[new_height - 1].state_proof.clone(),
            },
        };

        let new_attestation = Attestation {
            initial_state_root: exec_vars[new_height + i - 1].state_root,
            da_block_hash: [(new_height + i).try_into().unwrap(); 32].into(),
            post_state_root: exec_vars[new_height + i].state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: (new_height + i).try_into().unwrap(),
                proof: exec_vars[new_height + i - 1].state_proof.clone(),
            },
        };

        // Testing the transition invariant
        // We suppose that these values are always defined, otherwise we panic
        let last_height_attested = module
            .maximum_attested_height
            .get(&mut working_set)
            .expect("The maximum attested height should be set at genesis");

        // Update the max_attested_height in case the blocks have already been finalized
        let new_height_to_attest = last_height_attested + 1;

        let min_height = new_height_to_attest.saturating_sub(DEFAULT_ROLLUP_FINALITY);

        // We have to check the following order invariant is respected:
        // min_height <= bonding_proof.transition_num <= new_height_to_attest
        // If this invariant is respected, we can be sure that the attester was bonded at new_height_to_attest.
        let transition_num = old_attestation.proof_of_bond.claimed_transition_num;

        assert!(
            min_height <= transition_num,
            "The transition number {transition_num} should be above the minimum height {min_height}"
        );

        assert!(
            transition_num <= new_height_to_attest,
            "The transition number {transition_num} should be below the new max attested height {new_height_to_attest}"
        );

        module
            .process_attestation(&context, old_attestation.into(), &mut working_set)
            .expect("Should succeed");

        module
            .process_attestation(&context, new_attestation.into(), &mut working_set)
            .expect("Should succeed");
    }

    let finality_usize = usize::try_from(DEFAULT_ROLLUP_FINALITY).unwrap();

    // Now the transition invariant is no longer respected: the transition number is below the minimum height or above the max height
    let old_attestation = Attestation {
        initial_state_root: exec_vars[new_height].state_root,
        da_block_hash: [(new_height + finality_usize + 1).try_into().unwrap(); 32].into(),
        post_state_root: exec_vars[new_height + 1].state_root,
        proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
            claimed_transition_num: new_height.try_into().unwrap(),
            proof: exec_vars[new_height - 1].state_proof.clone(),
        },
    };

    // Testing the transition invariant
    // We suppose that these values are always defined, otherwise we panic
    let last_height_attested = module
        .maximum_attested_height
        .get(&mut working_set)
        .expect("The maximum attested height should be set at genesis");

    // Update the max_attested_height in case the blocks have already been finalized
    let new_height_to_attest = last_height_attested + 1;

    let min_height = new_height_to_attest.saturating_sub(DEFAULT_ROLLUP_FINALITY);

    let transition_num = old_attestation.proof_of_bond.claimed_transition_num;

    assert!(
        min_height > transition_num,
        "The transition number {transition_num} should now be below the minimum height {min_height}"
    );

    let err = module
        .process_attestation(&context, old_attestation.into(), &mut working_set)
        .unwrap_err();

    assert_eq!(
        err,
        AttesterIncentiveErrors::InvalidTransitionInvariant,
        "The transition invariant is not respected anymore"
    );

    // Now we do the same, except that the proof of bond refers to a transition above the transition to prove
    let attestation = Attestation {
        initial_state_root: exec_vars[new_height + finality_usize].state_root,
        da_block_hash: [(new_height + finality_usize + 1).try_into().unwrap(); 32].into(),
        post_state_root: exec_vars[new_height + finality_usize + 1].state_root,
        proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
            claimed_transition_num: (new_height + finality_usize + 2).try_into().unwrap(),
            proof: exec_vars[new_height + finality_usize + 1]
                .state_proof
                .clone(),
        },
    };

    let transition_num = attestation.proof_of_bond.claimed_transition_num;

    assert!(
        transition_num > new_height_to_attest,
        "The transition number {transition_num} should now be below the new height to attest {new_height_to_attest}"
    );

    let err = module
        .process_attestation(&context, attestation.into(), &mut working_set)
        .unwrap_err();

    assert_eq!(
        err,
        AttesterIncentiveErrors::InvalidTransitionInvariant,
        "The transition invariant is not respected anymore"
    );
}

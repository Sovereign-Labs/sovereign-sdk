use jmt::proof::SparseMerkleProof;
use sov_chain_state::StateTransitionId;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Address, Hasher, Module, Spec};
use sov_rollup_interface::mocks::{
    MockCodeCommitment, MockProof, MockZkvm, TestValidityCond, TestValidityCondChecker,
};
use sov_rollup_interface::optimistic::Attestation;
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker};
use sov_state::storage::{StorageKey, StorageProof, StorageValue};
use sov_state::{ArrayWitness, ProverStorage, Storage, WorkingSet};

use crate::helpers::{setup, BOND_AMOUNT, INITIAL_BOND_AMOUNT};
use crate::AttesterIncentives;

type C = DefaultContext;

const MOCK_CODE_COMMITMENT: MockCodeCommitment = MockCodeCommitment([0u8; 32]);
const DEFAULT_MAX_LIGHT_CLIENT_HEIGHT: u64 = 6;

/// Tests the different cases where an attestation is invalid and the
/// associated bond must be burnt away:
/// - incorrect maximum-attested-height
/// - incorrect initial-block-hash
#[test]
fn test_burn_on_invalid_attestation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, attester_address, _) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);

    // Assert that the attester has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                attester_address.clone(),
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    module
        .light_client_finalized_height
        .set(&DEFAULT_MAX_LIGHT_CLIENT_HEIGHT, &mut working_set);

    // Commit the working set
    let (reads_writes, witness) = working_set.checkpoint().freeze();

    storage
        .validate_and_commit(reads_writes, &witness)
        .expect("Should be able to commit");

    let mut working_set = WorkingSet::new(storage);

    // Process an invalid attestation
    {
        let context = DefaultContext {
            sender: attester_address.clone(),
        };

        let proof = module.get_bond_proof(
            attester_address.clone(),
            &ArrayWitness::default(),
            &mut working_set,
        );

        let storage_proof = StorageProof {
            key: StorageKey::new(module.bonded_attesters.prefix(), &attester_address),
            value: Some(BOND_AMOUNT.to_le_bytes().to_vec().into()),
            proof: proof.proof,
        };

        let attestation = Attestation {
            initial_state_root: [0; 32],
            da_block_hash: [0; 32],
            post_state_root: [0; 32],
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: 0,
                proof: storage_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is not an error");
    }

    // Assert that the prover's bond amount has been burned
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
}

#[test]
fn test_burn_on_invalid_proof() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, attester_address, challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);

    // Assert that the prover has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                attester_address.clone(),
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Process an invalid proof
    {
        let context = DefaultContext {
            sender: attester_address.clone(),
        };

        let proof = module.get_bond_proof(
            attester_address.clone(),
            &ArrayWitness::default(),
            &mut working_set,
        );

        let storage_proof = StorageProof {
            key: StorageKey::new(module.bonded_attesters.prefix(), &attester_address),
            value: Some(INITIAL_BOND_AMOUNT.to_le_bytes().to_vec().into()),
            proof: proof.proof,
        };

        let attestation = Attestation {
            initial_state_root: [0; 32],
            da_block_hash: [0; 32],
            post_state_root: [0; 32],
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: 0,
                proof: storage_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is not an error");
    }

    // Assert that the prover's bond amount has been burned
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
}

#[test]
fn test_valid_proof() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, attester_address, challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);

    // Assert that the prover has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                attester_address.clone(),
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Process a valid proof
    {
        let context = DefaultContext {
            sender: attester_address.clone(),
        };

        let proof = module.get_bond_proof(
            attester_address.clone(),
            &ArrayWitness::default(),
            &mut working_set,
        );

        let storage_proof = StorageProof {
            key: StorageKey::new(module.bonded_attesters.prefix(), &attester_address.clone()),
            value: Some(INITIAL_BOND_AMOUNT.to_le_bytes().to_vec().into()),
            proof: proof.proof,
        };

        let attestation = Attestation {
            initial_state_root: [0; 32],
            da_block_hash: [0; 32],
            post_state_root: [0; 32],
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: 0,
                proof: storage_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is not an error");
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
}

#[test]
fn test_unbonding() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, attester_address, challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);
    let context = DefaultContext {
        sender: attester_address.clone(),
    };
    let token_address = module
        .bonding_token_address
        .get(&mut working_set)
        .expect("bonding token address was set at genesis");

    // Assert that the prover has bonded tokens
    assert_eq!(
        module
            .get_bond_amount(
                attester_address.clone(),
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    // Get their *unlocked* balance before undbonding
    // let initial_unlocked_balance = {
    //     module
    //         .bank
    //         .get_balance_of(
    //             attester_address.clone(),
    //             token_address.clone(),
    //             &mut working_set,
    //         )
    //         .unwrap_or_default()
    // };

    // Unbond the prover
    module
        .unbond_challenger(&context, &mut working_set)
        .expect("Unbonding should succeed");

    // Assert that the prover no longer has bonded tokens
    assert_eq!(
        module
            .get_bond_amount(
                attester_address.clone(),
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        0
    );

    // Assert that the prover's unlocked balance has increased by the amount they unbonded
    // let unlocked_balance =
    //     module
    //         .bank
    //         .get_balance_of(attester_address, token_address, &mut working_set);
    // assert_eq!(
    //     unlocked_balance,
    //     Some(BOND_AMOUNT + initial_unlocked_balance)
    // );
}

#[test]
fn test_prover_not_bonded() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, attester_address, challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);
    let context = DefaultContext {
        sender: attester_address.clone(),
    };

    // Unbond the prover
    module
        .unbond_challenger(&context, &mut working_set)
        .expect("Unbonding should succeed");

    // Assert that the prover no longer has bonded tokens
    assert_eq!(
        module
            .get_bond_amount(
                attester_address.clone(),
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        0
    );

    // Process a valid proof
    {
        let proof = module.get_bond_proof(
            attester_address.clone(),
            &ArrayWitness::default(),
            &mut working_set,
        );

        let storage_proof = StorageProof {
            key: StorageKey::new(module.bonded_attesters.prefix(), &attester_address),
            value: Some(INITIAL_BOND_AMOUNT.to_le_bytes().to_vec().into()),
            proof: proof.proof,
        };

        let attestation = Attestation {
            initial_state_root: [0; 32],
            da_block_hash: [0; 32],
            post_state_root: [0; 32],
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: 0,
                proof: storage_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is not an error");
    }
}

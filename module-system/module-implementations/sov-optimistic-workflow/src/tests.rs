use jmt::proof::SparseMerkleProof;
use sov_chain_state::StateTransitionId;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Hasher, Module, Spec};
use sov_rollup_interface::mocks::{
    MockCodeCommitment, MockProof, MockZkvm, TestValidityCond, TestValidityCondChecker,
};
use sov_rollup_interface::optimistic::Attestation;
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker};
use sov_state::storage::{StorageKey, StorageProof, StorageValue};
use sov_state::{ArrayWitness, ProverStorage, Storage, WorkingSet};

use crate::AttesterIncentives;

type C = DefaultContext;

const BOND_AMOUNT: u64 = 1000;
const INITIAL_BOND_AMOUNT: u64 = 5 * BOND_AMOUNT;
const MOCK_CODE_COMMITMENT: MockCodeCommitment = MockCodeCommitment([0u8; 32]);
const DEFAULT_ROLLUP_FINALITY: u64 = 3;
const DEFAULT_CHAIN_HEIGHT: u64 = 12;
const DEFAULT_MAX_LIGHT_CLIENT_HEIGHT: u64 = 6;

pub fn generate_address(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    Address::from(hash)
}

fn create_bank_config() -> (
    sov_bank::BankConfig<C>,
    <C as Spec>::Address,
    <C as Spec>::Address,
) {
    let attester_address = generate_address("attester_pub_key");
    let challenger_address = generate_address("challenger_pub_key");

    let token_config = sov_bank::TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances: vec![
            (attester_address.clone(), INITIAL_BOND_AMOUNT),
            (challenger_address.clone(), INITIAL_BOND_AMOUNT),
        ],
    };

    (
        sov_bank::BankConfig {
            tokens: vec![token_config],
        },
        attester_address,
        challenger_address,
    )
}

fn setup<Cond: ValidityCondition, Checker: ValidityConditionChecker<Cond>>(
    working_set: &mut WorkingSet<<C as Spec>::Storage>,
) -> (
    AttesterIncentives<C, MockZkvm, Cond, Checker>,
    Address,
    Address,
) {
    // Initialize bank
    let (bank_config, attester_address, challenger_address) = create_bank_config();
    let bank = sov_bank::Bank::<C>::default();
    bank.genesis(&bank_config, working_set)
        .expect("bank genesis must succeed");

    let token_address = sov_bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &sov_bank::genesis::DEPLOYER,
        sov_bank::genesis::SALT,
    );

    // We don't need to initialize the chain state as there is no genesis for that module

    // initialize prover incentives
    let module = AttesterIncentives::<C, MockZkvm, Cond, Checker>::default();
    let config = crate::AttesterIncentivesConfig {
        bonding_token_address: token_address,
        minimum_attester_bond: BOND_AMOUNT,
        minimum_challenger_bond: BOND_AMOUNT,
        commitment_to_allowed_challenge_method: MockCodeCommitment([0u8; 32]),
        initial_attesters: vec![(attester_address.clone(), BOND_AMOUNT)],
        rollup_finality_period: DEFAULT_ROLLUP_FINALITY,
    };

    module
        .genesis(&config, working_set)
        .expect("prover incentives genesis must succeed");
    (module, attester_address, challenger_address)
}

fn init_chain(
    module: &AttesterIncentives<
        DefaultContext,
        MockZkvm,
        TestValidityCond,
        TestValidityCondChecker<TestValidityCond>,
    >,
    working_set: &mut WorkingSet<<C as Spec>::Storage>,
) {
    // Initialize the chain state with some values
    module.chain_state.historical_transitions.set(
        &0,
        &StateTransitionId::new([0; 32], [0; 32], TestValidityCond::default()),
        working_set,
    );

    module
        .chain_state
        .slot_height
        .set(&DEFAULT_CHAIN_HEIGHT, working_set);

    for i in 0..DEFAULT_CHAIN_HEIGHT {
        let i_u8 = u8::try_from(i).unwrap();
        let validity_condition = TestValidityCond::default();
        let state_tx = StateTransitionId::<TestValidityCond>::new(
            [10_u8 + i_u8; 32],
            [i_u8; 32],
            validity_condition,
        );
        module
            .chain_state
            .historical_transitions
            .set(&i, &state_tx, working_set)
    }
}

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

    // We have to initialize the chain state
    init_chain(&module, &mut working_set);

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

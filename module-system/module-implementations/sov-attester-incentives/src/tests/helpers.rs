use jmt::proof::SparseMerkleProof;
use sov_bank::{BankConfig, TokenConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::SlotHooks;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Address, Genesis, Spec, ValidityConditionChecker};
use sov_rollup_interface::mocks::{
    MockBlock, MockBlockHeader, MockCodeCommitment, MockDaSpec, MockValidityCond,
    MockValidityCondChecker, MockZkvm,
};
use sov_state::storage::StorageProof;
use sov_state::{DefaultStorageSpec, ProverStorage, Storage, WorkingSet};

use crate::AttesterIncentives;

type C = DefaultContext;

pub const TOKEN_NAME: &str = "TEST_TOKEN";
pub const BOND_AMOUNT: u64 = 1000;
pub const INITIAL_BOND_AMOUNT: u64 = 5 * BOND_AMOUNT;
pub const SALT: u64 = 5;
pub const DEFAULT_ROLLUP_FINALITY: u64 = 3;
pub const INIT_HEIGHT: u64 = 0;

/// Consumes and commit the existing working set on the underlying storage
/// `storage` must be the underlying storage defined on the working set for this method to work.
pub(crate) fn commit_get_new_working_set(
    storage: &ProverStorage<DefaultStorageSpec>,
    working_set: WorkingSet<<C as Spec>::Storage>,
) -> WorkingSet<<C as Spec>::Storage> {
    let (reads_writes, witness) = working_set.checkpoint().freeze();

    storage
        .validate_and_commit(reads_writes, &witness)
        .expect("Should be able to commit");

    WorkingSet::new(storage.clone())
}

pub(crate) fn create_bank_config_with_token(
    token_name: String,
    salt: u64,
    addresses_count: usize,
    initial_balance: u64,
) -> (BankConfig<C>, Vec<Address>) {
    let address_and_balances: Vec<(Address, u64)> = (0..addresses_count)
        .map(|i| {
            let key = format!("key_{}", i);
            let addr = generate_address::<C>(&key);
            (addr, initial_balance)
        })
        .collect();

    let token_config = TokenConfig {
        token_name,
        address_and_balances: address_and_balances.clone(),
        authorized_minters: vec![address_and_balances.first().unwrap().0],
        salt,
    };

    (
        BankConfig {
            tokens: vec![token_config],
        },
        address_and_balances
            .into_iter()
            .map(|(addr, _)| addr)
            .collect(),
    )
}

/// Creates a bank config with a token, and a prover incentives module.
/// Returns the prover incentives module and the attester and challenger's addresses.
pub(crate) fn setup(
    working_set: &mut WorkingSet<<C as Spec>::Storage>,
) -> (
    AttesterIncentives<C, MockZkvm, MockDaSpec, MockValidityCondChecker<MockValidityCond>>,
    Address,
    Address,
    Address,
) {
    // Initialize bank
    let (bank_config, mut addresses) =
        create_bank_config_with_token(TOKEN_NAME.to_string(), SALT, 3, INITIAL_BOND_AMOUNT);
    let bank = sov_bank::Bank::<C>::default();
    bank.genesis(&bank_config, working_set)
        .expect("bank genesis must succeed");

    let attester_address = addresses.pop().unwrap();
    let challenger_address = addresses.pop().unwrap();
    let reward_supply = addresses.pop().unwrap();

    let token_address = sov_bank::get_genesis_token_address::<DefaultContext>(TOKEN_NAME, SALT);

    // Initialize chain state
    let chain_state_config = sov_chain_state::ChainStateConfig {
        initial_slot_height: INIT_HEIGHT,
    };

    let chain_state = sov_chain_state::ChainState::<C, MockDaSpec>::default();
    chain_state
        .genesis(&chain_state_config, working_set)
        .expect("Chain state genesis must succeed");

    // initialize prover incentives
    let module = AttesterIncentives::<
        C,
        MockZkvm,
        MockDaSpec,
        MockValidityCondChecker<MockValidityCond>,
    >::default();
    let config = crate::AttesterIncentivesConfig {
        bonding_token_address: token_address,
        reward_token_supply_address: reward_supply,
        minimum_attester_bond: BOND_AMOUNT,
        minimum_challenger_bond: BOND_AMOUNT,
        commitment_to_allowed_challenge_method: MockCodeCommitment([0u8; 32]),
        initial_attesters: vec![(attester_address, BOND_AMOUNT)],
        rollup_finality_period: DEFAULT_ROLLUP_FINALITY,
        maximum_attested_height: INIT_HEIGHT,
        light_client_finalized_height: INIT_HEIGHT,
        validity_condition_checker: MockValidityCondChecker::<MockValidityCond>::new(),
        phantom_data: Default::default(),
    };

    module
        .genesis(&config, working_set)
        .expect("prover incentives genesis must succeed");

    (module, token_address, attester_address, challenger_address)
}

pub(crate) struct ExecutionSimulationVars {
    pub state_root: [u8; 32],
    pub state_proof: StorageProof<SparseMerkleProof<<C as Spec>::Hasher>>,
}

/// Generate an execution simulation for a given number of rounds. Returns a list of the successive state roots
/// with associated bonding proofs, as long as the last state root
pub(crate) fn execution_simulation<Checker: ValidityConditionChecker<MockValidityCond>>(
    rounds: u8,
    module: &AttesterIncentives<C, MockZkvm, MockDaSpec, Checker>,
    storage: &ProverStorage<DefaultStorageSpec>,
    attester_address: <C as Spec>::Address,
    mut working_set: WorkingSet<<C as Spec>::Storage>,
) -> (
    // Vector of the successive state roots with associated bonding proofs
    Vec<ExecutionSimulationVars>,
    WorkingSet<<C as Spec>::Storage>,
) {
    let mut ret_exec_vars = Vec::<ExecutionSimulationVars>::new();

    for i in 0..rounds {
        // Commit the working set
        working_set = commit_get_new_working_set(storage, working_set);

        ret_exec_vars.push(ExecutionSimulationVars {
            state_root: storage.get_state_root(&Default::default()).unwrap(),
            state_proof: module.get_bond_proof(
                attester_address,
                &Default::default(),
                &mut working_set,
            ),
        });

        // Then process the first transaction. Only sets the genesis hash and a transition in progress.
        let slot_data = MockBlock {
            header: MockBlockHeader {
                prev_hash: [i; 32].into(),
                hash: [i + 1; 32].into(),
            },
            height: INIT_HEIGHT + u64::from(i + 1),
            validity_cond: MockValidityCond { is_valid: true },
            blobs: Default::default(),
        };
        module.chain_state.begin_slot_hook(
            &slot_data.header,
            &slot_data.validity_cond,
            &mut working_set,
        );
    }

    (ret_exec_vars, working_set)
}

use sov_bank::{BankConfig, TokenConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Address, Genesis, Spec};
use sov_rollup_interface::mocks::{MockCodeCommitment, MockZkvm, TestValidityCond};
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker};
use sov_state::{DefaultStorageSpec, ProverStorage, Storage, WorkingSet};

use crate::AttesterIncentives;

type C = DefaultContext;

pub const TOKEN_NAME: &str = "TEST_TOKEN";
pub const BOND_AMOUNT: u64 = 1000;
pub const DEFAULT_CHAIN_HEIGHT: u64 = 12;
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
        authorized_minters: vec![],
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
pub(crate) fn setup<Cond: ValidityCondition, Checker: ValidityConditionChecker<Cond>>(
    working_set: &mut WorkingSet<<C as Spec>::Storage>,
) -> (
    AttesterIncentives<C, MockZkvm, Cond, Checker>,
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

    let chain_state = sov_chain_state::ChainState::<C, TestValidityCond>::default();
    chain_state
        .genesis(&chain_state_config, working_set)
        .expect("Chain state genesis must succeed");

    // initialize prover incentives
    let module = AttesterIncentives::<C, MockZkvm, Cond, Checker>::default();
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
    };

    module
        .genesis(&config, working_set)
        .expect("prover incentives genesis must succeed");

    (module, token_address, attester_address, challenger_address)
}

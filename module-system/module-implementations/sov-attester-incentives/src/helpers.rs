use sov_bank::{BankConfig, TokenConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Address, Genesis, Spec};
use sov_rollup_interface::mocks::{MockCodeCommitment, MockZkvm};
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker};
use sov_state::WorkingSet;

use crate::AttesterIncentives;

type C = DefaultContext;

pub const TOKEN_NAME: &str = "TEST_TOKEN";
pub const BOND_AMOUNT: u64 = 1000;
pub const DEFAULT_CHAIN_HEIGHT: u64 = 12;
pub const INITIAL_BOND_AMOUNT: u64 = 5 * BOND_AMOUNT;
pub const SALT: u64 = 5;
pub const DEFAULT_ROLLUP_FINALITY: u64 = 3;

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
) {
    // Initialize bank
    let (bank_config, mut addresses) =
        create_bank_config_with_token(TOKEN_NAME.to_string(), SALT, 2, INITIAL_BOND_AMOUNT);
    let bank = sov_bank::Bank::<C>::default();
    bank.genesis(&bank_config, working_set)
        .expect("bank genesis must succeed");

    let attester_address = addresses.pop().unwrap();
    let challenger_address = addresses.pop().unwrap();

    let token_address = sov_bank::get_genesis_token_address::<DefaultContext>(TOKEN_NAME, SALT);

    // initialize prover incentives
    let module = AttesterIncentives::<C, MockZkvm, Cond, Checker>::default();
    let config = crate::AttesterIncentivesConfig {
        bonding_token_address: token_address,
        minimum_attester_bond: BOND_AMOUNT,
        minimum_challenger_bond: BOND_AMOUNT,
        commitment_to_allowed_challenge_method: MockCodeCommitment([0u8; 32]),
        initial_attesters: vec![(attester_address, BOND_AMOUNT)],
        rollup_finality_period: DEFAULT_ROLLUP_FINALITY,
    };

    module
        .genesis(&config, working_set)
        .expect("prover incentives genesis must succeed");
    (module, attester_address, challenger_address)
}

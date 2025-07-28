use sov_modules_api::{Amount, Spec};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;

use crate::helpers::{GenesisConfig, RT, S};

#[test]
#[should_panic(expected = "The total_supply:")]
pub fn test_supply_cap_too_small() {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    // If supply_cap is set too low, total_supply will exceed it, resulting in a panic at initialization.
    genesis
        .bank
        .gas_token_config
        .address_and_balances
        .push((<S as Spec>::Address::new([11; 28]), Amount::new(1000)));
    genesis.bank.gas_token_config.supply_cap = Some(Amount::new(10));

    let runtime = RT::default();
    TestRunner::<RT, S>::new_with_genesis(genesis.into_genesis_params(), runtime);
}

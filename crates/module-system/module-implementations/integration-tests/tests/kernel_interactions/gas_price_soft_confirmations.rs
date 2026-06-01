use sov_chain_state::ChainState;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::GasSpec;
use sov_rollup_interface::common::IntoSlotNumber;
use sov_test_utils::generate_optimistic_runtime_with_kernel;

use crate::kernel_interactions::{HighLevelOptimisticGenesisConfig, TestRunner, S};

generate_optimistic_runtime_with_kernel!(TestGasPriceRuntime <= kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>, modules: [],);

type RT = TestGasPriceRuntime<S>;

#[test]
fn test_gas_price_soft_confirmations() {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());

    let mut runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let chain_state = ChainState::<S>::default();

    let initial_gas_price = runner.query_visible_state(|state| {
        chain_state
            .base_fee_per_gas(state)
            .unwrap_infallible()
            .unwrap()
    });

    assert_eq!(initial_gas_price, S::initial_base_fee_per_gas());

    runner.advance_slots(1);

    let next_gas_price = runner.query_visible_state(|state| {
        let next_gas_price = chain_state
            .base_fee_per_gas(state)
            .unwrap_infallible()
            .unwrap();

        assert_eq!(
            next_gas_price, initial_gas_price,
            "The next gas price is the one for the slot after the last visible slot"
        );

        next_gas_price
    });

    runner.advance_slots(1);

    runner.query_visible_state(|state| {
        assert_eq!(
            chain_state
                .base_fee_per_gas(state)
                .unwrap_infallible()
                .unwrap(),
            next_gas_price,
            "The gas price should not have changed because the visible slot height has not changed"
        );
    });

    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT") - 2);

    runner.query_visible_state(|state| {
        assert_eq!(
            chain_state
                .base_fee_per_gas(state)
                .unwrap_infallible()
                .unwrap(),
            next_gas_price,
            "The gas price should not have changed because the visible slot height has not changed"
        );
    });

    runner.advance_slots(1);

    // The gas price should have changed because the visible slot height has changed
    let next_gas_price = runner.query_visible_state(|state| {
        let next_gas_price = chain_state
            .base_fee_per_gas(state)
            .unwrap_infallible()
            .unwrap();

        assert_ne!(
            next_gas_price, initial_gas_price,
            "The next gas price is not the one for the slot after the last visible slot"
        );

        assert_eq!(
            next_gas_price,
            ChainState::<S>::compute_base_fee_per_gas(
                chain_state
                    .slot_at_height(1.to_slot_number(), state)
                    .unwrap_infallible()
                    .unwrap()
                    .gas_info()
                    .clone(),
                1,
            ),
            "The gas price should be the one for the slot after the last visible slot"
        );

        next_gas_price
    });

    runner.advance_slots(1);

    runner.query_visible_state(|state| {
        assert_ne!(
            chain_state
                .base_fee_per_gas(state)
                .unwrap_infallible()
                .unwrap(),
            next_gas_price,
            "The gas price should have changed because the visible slot height has changed"
        );
    });
}

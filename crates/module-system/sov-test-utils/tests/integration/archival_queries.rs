use sov_chain_state::ChainState;
use sov_kernels::soft_confirmations::SoftConfirmationsKernel;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::VersionReader;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{generate_optimistic_runtime_with_kernel, TestSpec};

generate_optimistic_runtime_with_kernel!(
    TestRuntimeWithSoftConfirmations <= kernel_type: SoftConfirmationsKernel<'a, S>, modules: [],
);

pub type S = TestSpec;

/// Sets up a test runner with the [`ValueSetter`] with a single additional admin account.
pub fn setup_soft_confirmations() -> TestRunner<TestRuntimeWithSoftConfirmations<S>, S> {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    // Run genesis registering the attester and sequencer we've generated.
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());

    TestRunner::new_with_genesis(
        genesis.into_genesis_params(),
        TestRuntimeWithSoftConfirmations::default(),
    )
}

#[test]
fn test_query_visible_state_soft_confirmations() {
    let mut runner = setup_soft_confirmations();

    let genesis_time = runner.query_visible_state(|state| {
        ChainState::<S>::default()
            .get_time(state)
            .unwrap_infallible()
    });

    runner.advance_slots(1);

    // We now query the visible state at the current height, we should see the genesis state because the visible state is not updated.
    runner.query_visible_state(|state| {
        assert_eq!(
            state.current_visible_slot_number().get(),
            0,
            "The visible state should be at the genesis height"
        );

        assert_eq!(
            sov_chain_state::ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible(),
            genesis_time,
            "The time should not have been updated"
        );
    });

    // We now query the true state at the current height, we should see the updated state.
    runner.query_state(|state| {
        assert_eq!(
            state.current_visible_slot_number().get(),
            1,
            "The true state should be higher than genesis"
        );

        assert_eq!(
            ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible(),
            None,
            "The value should still not be set because the batch is deferred"
        );

        assert_ne!(
            sov_chain_state::ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible(),
            genesis_time,
            "The time should have been updated"
        );
    });

    // We advance the slots to ensure that the visible hash is updated
    runner.advance_slots(1);

    // We now query the visible state at the current height, we should still see the genesis state because the visible state is not updated.
    runner.query_visible_state(|state| {
        assert_eq!(
            state.current_visible_slot_number().get(),
            0,
            "The value should be set to 0"
        );

        assert_eq!(
            sov_chain_state::ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible(),
            genesis_time,
            "The time should not have been updated"
        );
    });

    // We advance the slots to be behind by at least `DEFERRED_SLOTS_COUNT` to
    // ensure that the visible state is updated.
    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT") - 2);

    // We now query the visible state at the current height, the visible state should be updated.
    runner.query_visible_state(|state| {
        assert_eq!(state.current_visible_slot_number().get(), 1);

        assert_ne!(
            sov_chain_state::ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible(),
            genesis_time
        );
    });

    runner.query_state_at_height(
        RollupHeight::new(runner.true_slot_number().get() - 1),
        |state| {
            assert_eq!(state.current_visible_slot_number().get(), 0);

            assert_eq!(
                sov_chain_state::ChainState::<S>::default()
                    .get_time(state)
                    .unwrap_infallible(),
                genesis_time
            );
        },
    );
}

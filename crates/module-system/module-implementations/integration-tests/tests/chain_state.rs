use sov_chain_state::ChainState;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{GetGasPrice, VersionReader};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_optimistic_runtime, get_gas_used, AsUser, TestSpec, TestUser};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

type S = sov_test_utils::TestSpec;

generate_optimistic_runtime!(TestKernelUpdatesRuntime <= value_setter: ValueSetter<S>);
type RT = TestKernelUpdatesRuntime<TestSpec>;

fn setup() -> (TestUser<S>, TestRunner<TestKernelUpdatesRuntime<S>, S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
    );

    let runner = TestRunner::new_with_genesis(
        genesis.into_genesis_params(),
        TestKernelUpdatesRuntime::default(),
    );

    (admin, runner)
}

#[test]
fn chain_state_kernel_updates_basic_kernel() {
    let (admin, mut runner) = setup();

    runner.query_state(|state| {
        assert_eq!(
            state.current_visible_slot_number(),
            VisibleSlotNumber::GENESIS,
            "The kernel should be initialized to zero"
        );
    });

    runner.query_visible_state(|state| {
        assert_eq!(
            state.current_visible_slot_number(),
            VisibleSlotNumber::GENESIS,
            "The kernel visible slot should be initialized to zero"
        );
    });

    runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    runner.query_state(|state| {
        assert_eq!(
            state.current_visible_slot_number(),
            VisibleSlotNumber::ONE,
            "The kernel should be updated to one"
        );
    });

    runner.query_visible_state(|state| {
        assert_eq!(
            state.current_visible_slot_number(),
            VisibleSlotNumber::ONE,
            "The kernel visible slot should be updated to one"
        );
    });
}

#[test]
fn test_chain_state_gas_updates() {
    let (admin, mut runner) = setup();

    let genesis_state_root = *runner.state_root();

    let output = runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    runner.query_state(|kernel| {
        assert_eq!(
            ChainState::<S>::default().get_genesis_hash(kernel).unwrap(),
            Some(genesis_state_root),
            "The genesis hash should be set"
        );

        let gas_consumed = get_gas_used(&output.0.batch_receipts[0].tx_receipts[0]);

        let in_progress_transition = ChainState::<S>::default()
            .latest_visible_slot(kernel)
            .unwrap_infallible()
            .unwrap();

        assert_eq!(
            in_progress_transition.gas_used(),
            &gas_consumed,
            "The gas consumed should be set"
        );
    });
}

#[test]
fn test_chain_state_root_updates() {
    let (admin, mut runner) = setup();

    let genesis_state_root = *runner.state_root();

    runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    let post_state_root = *runner.state_root();

    runner.query_state(|kernel| {
        assert_eq!(
            ChainState::<S>::default().get_genesis_hash(kernel).unwrap(),
            Some(genesis_state_root),
            "The genesis hash should be set"
        );
    });

    runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    runner.query_state(|kernel| {
        let first_transition = ChainState::<S>::default()
            .get_historical_transition_dangerous(SlotNumber::ONE, kernel)
            .unwrap_infallible()
            .unwrap();

        assert_eq!(
            first_transition.post_state_root(),
            &post_state_root,
            "The post state root should be set"
        );
    });
}

#[test]
fn test_chain_state_historical_transition_update() {
    let (admin, mut runner) = setup();

    runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    let in_progress_transition = runner.query_state(|kernel| {
        ChainState::<S>::default()
            .latest_visible_slot(kernel)
            .unwrap_infallible()
            .unwrap()
    });

    runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    runner.query_state(|kernel| {
        let first_transition = ChainState::<S, >::default()
            .slot_at_height(SlotNumber::ONE, kernel)
            .unwrap_infallible()
            .unwrap();

        assert_eq!(
            in_progress_transition.slot_hash(),
            first_transition.slot_hash(),
            "The slot hashes of the in progress and the first historical transition should be the same"
        );

        assert_eq!(
            in_progress_transition.gas_used(),
            first_transition.gas_used(),
            "The gas used of the in progress and the first historical transition should be the same"
        );
    });
}

/// This test ensures that the gas price for the archival state updates correctly
/// when a previous state is queried.
#[test]
fn test_archival_state_updates_gas_price() {
    let (admin, mut runner) = setup();

    let initial_base_fee_per_gas = runner
        .query_visible_state(|state| {
            ChainState::<S>::default()
                .base_fee_per_gas(state)
                .unwrap_infallible()
        })
        .unwrap();

    runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    runner.advance_slots(1);

    let current_gas_price = runner
        .query_visible_state(|state| {
            ChainState::<S>::default()
                .base_fee_per_gas(state)
                .unwrap_infallible()
        })
        .unwrap();

    assert_ne!(
        initial_base_fee_per_gas, current_gas_price,
        "The gas price should have changed"
    );

    runner.query_visible_state(|state| {
        let gas_price = state.gas_price();

        assert_eq!(
            gas_price, &current_gas_price,
            "The gas price stored in the accessor should be the same as the current gas price"
        );

        let archival_state = state.get_archival_state(RollupHeight::new(1)).unwrap();

        assert_eq!(
            archival_state.gas_price(),
            &initial_base_fee_per_gas,
            "The gas price stored in the archival state should be the same as the initial gas price"
        );
    });
}

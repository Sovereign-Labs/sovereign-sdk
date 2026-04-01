use sov_chain_state::ChainState;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::da::Time;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_test_utils::{AsUser, TransactionTestCase};
use sov_value_setter::ValueSetter;

use crate::helpers::{setup, RT, S};

/// Ensures that [`TestRunner::query_visible_state`] returns an [`sov_modules_api::ApiStateAccessor`] on the latest (most recent) state.
#[test]
fn test_query_runtime() {
    let (admin, mut runner) = setup();

    let admin_genesis_address = runner.query_visible_state(|state| {
        assert_eq!(
            ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible(),
            None,
            "The value should not be set"
        );

        ValueSetter::<S>::default()
            .admin
            .get(state)
            .unwrap_infallible()
            .expect("The admin should be set")
    });

    assert_eq!(
        admin.address(),
        admin_genesis_address,
        "The admins don't match"
    );

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 1,
                gas: None,
            },
        ),
        assert: Box::new(move |_result, state| {
            let value = ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible();
            assert_eq!(value, Some(1), "The value should be set to 1");
        }),
    });
}

/// Ensures that calling [`TestRunner::query_archival_state`] returns an [`sov_modules_api::ApiStateAccessor`] on an archived (outdated) state.
#[test]
fn test_query_archival_state() {
    let (admin, mut runner) = setup();

    runner.query_visible_state(|state| {
        assert_eq!(
            ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible(),
            None,
            "The value should not be set"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 1,
                gas: None,
            },
        ),
        assert: Box::new(move |_result, state| {
            let value = ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible();
            assert_eq!(value, Some(1), "The value should be set to 1");
        }),
    });

    runner.query_state_at_height(RollupHeight::new(0), |state| {
        assert_eq!(
            ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible(),
            None,
            "The value should not be set"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 2,
                gas: None,
            },
        ),
        assert: Box::new(move |_result, state| {
            let value = ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible();
            assert_eq!(value, Some(2), "The value should be set to 1");
        }),
    });

    runner.query_state_at_height(RollupHeight::new(1), |state| {
        assert_eq!(
            ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible(),
            Some(1),
            "The value was set to 1 at height 1"
        );
    });
}

/// Ensures that archival queries return correct values for both user state
/// ([`StateValue`]) and accessory state ([`AccessoryStateValue`]) at each height.
#[test]
fn test_query_archival_state_user_and_accessory() {
    let (admin, mut runner) = setup();

    // Genesis (height 0): value is unset. finalize_hook_count is 1
    // because the genesis slot runs the finalize hook once.
    runner.query_state_at_height(RollupHeight::new(0), |state| {
        assert_eq!(
            ValueSetter::<S>::default()
                .value
                .get(state)
                .unwrap_infallible(),
            None,
            "User state: value should be unset at genesis"
        );
        assert_eq!(
            ValueSetter::<S>::default()
                .finalize_hook_count
                .get(state)
                .unwrap_infallible(),
            Some(1),
            "Accessory state: finalize_hook_count should be 1 after genesis slot"
        );
    });

    // Height 1: set value to 99. Finalize hook runs once.
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 99,
                gas: None,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Height 2: set value to 200. Finalize hook runs again.
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 200,
                gas: None,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Archival query at height 0: value unset, finalize_hook_count = 1 (genesis slot).
    runner.query_state_at_height(RollupHeight::new(0), |state| {
        let vs = ValueSetter::<S>::default();
        assert_eq!(
            vs.value.get(state).unwrap_infallible(),
            None,
            "User state at height 0 should still be unset"
        );
        assert_eq!(
            vs.finalize_hook_count.get(state).unwrap_infallible(),
            Some(1),
            "Accessory state at height 0 should be 1"
        );
    });

    // Archival query at height 1: value = 99, finalize_hook_count = 2.
    runner.query_state_at_height(RollupHeight::new(1), |state| {
        let vs = ValueSetter::<S>::default();
        assert_eq!(
            vs.value.get(state).unwrap_infallible(),
            Some(99),
            "User state at height 1 should be 99"
        );
        assert_eq!(
            vs.finalize_hook_count.get(state).unwrap_infallible(),
            Some(2),
            "Accessory state at height 1 should be 2"
        );
    });

    // Archival query at height 2: value = 200, finalize_hook_count = 3.
    runner.query_state_at_height(RollupHeight::new(2), |state| {
        let vs = ValueSetter::<S>::default();
        assert_eq!(
            vs.value.get(state).unwrap_infallible(),
            Some(200),
            "User state at height 2 should be 200"
        );
        assert_eq!(
            vs.finalize_hook_count.get(state).unwrap_infallible(),
            Some(3),
            "Accessory state at height 2 should be 3"
        );
    });
}

#[test]
fn test_freeze_time() {
    let (admin, mut runner) = setup();
    let chain_state = ChainState::<S>::default();

    runner.config.freeze_time = Some(Time::from_secs(200));
    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());
    // not frozen until the next slot.
    assert_ne!(time, Time::from_secs(200));

    runner.advance_slots(1);

    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());
    assert_eq!(time, Time::from_secs(200));
    let oracle_time =
        runner.query_state(|state| chain_state.get_oracle_time(state).unwrap_infallible());
    assert_eq!(oracle_time, Time::from_secs(200));

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 1,
                gas: None,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    let oracle_time =
        runner.query_state(|state| chain_state.get_oracle_time(state).unwrap_infallible());
    assert_eq!(
        oracle_time,
        Time::from_secs(200),
        "Oracle time should remain frozen during tx execution"
    );

    runner.advance_slots(1);
    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());
    // time is still frozen
    assert_eq!(time, Time::from_secs(200));

    runner.config.freeze_time = Some(Time::from_secs(5000));
    // time is still frozen in the current slot, as it is not advanced.
    assert_eq!(time, Time::from_secs(200));

    runner.advance_slots(1);

    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());
    // frozen time is updated
    assert_eq!(time, Time::from_secs(5000));
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 2,
                gas: None,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });
    let oracle_time =
        runner.query_state(|state| chain_state.get_oracle_time(state).unwrap_infallible());
    assert_eq!(
        oracle_time,
        Time::from_secs(5000),
        "Oracle time should remain frozen after changing freeze_time"
    );

    // timestamps should revert to the current time
    runner.config.freeze_time = None;
    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());
    // frozen time is still frozen until next slot
    assert_eq!(time, Time::from_secs(5000));

    runner.advance_slots(1);

    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());

    assert!(
        Time::from_secs(5000) < time,
        "Time should no longer be frozen"
    );
}

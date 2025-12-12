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

#[test]
fn test_freeze_time() {
    let (_, mut runner) = setup();
    let chain_state = ChainState::<S>::default();

    runner.config.freeze_time = Some(Time::from_secs(200));
    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());
    // not frozen until the next slot.
    assert_ne!(time, Time::from_secs(200));

    runner.advance_slots(1);

    let time = runner.query_state(|state| chain_state.get_time(state).unwrap_infallible());
    assert_eq!(time, Time::from_secs(200));

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

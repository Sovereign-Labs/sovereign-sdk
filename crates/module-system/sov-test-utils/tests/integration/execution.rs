use sov_mock_da::{MockAddress, MockDaSpec};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{DaSpec, Gas, GetGasPrice, Spec};
use sov_sequencer_registry::SequencerRegistry;
use sov_test_utils::{AsUser, BatchTestCase, TestSequencer};
use sov_value_setter::ValueSetter;

use crate::helpers::{setup, RT, S};

/// Tests that the batch is rewarded if the default sequencer is used
#[test]
fn test_default_sequencer() {
    let (admin, mut runner) = setup();

    runner.execute_batch(BatchTestCase {
        input: vec![admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 1,
                gas: None,
            },
        )]
        .into(),
        assert: Box::new(move |result, _state| {
            assert_eq!(result.sender_da_address, runner.config.sequencer_da_address);
        }),
    });
}

/// Tests that the batch is dropped if the specified sequencer is not registered
#[test]
fn test_specify_non_default_sequencer_errors_if_not_registered() {
    let (admin, mut runner) = setup();

    runner.config.sequencer_da_address = <MockDaSpec as DaSpec>::Address::from([42; 32]);

    runner.execute_batch(BatchTestCase {
        input: vec![admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 10,
                gas: None,
            },
        )]
        .into(),
        assert: Box::new(move |result, _state| {
            assert!(
                result.batch_receipt.is_none(),
                "Batch should have been dropped"
            );
        }),
    });
}

/// Tests that we can register and use another sequencer
#[test]
fn test_register_sequencer() {
    let (additional_user, mut runner) = setup();

    let new_sequencer_address = MockAddress::from([42; 32]);

    let user_stake_value = runner.query_visible_state(|state| {
        <S as Spec>::Gas::from(config_value!("MAX_SEQUENCER_EXEC_GAS_PER_TX"))
            .value(state.gas_price())
    });

    let new_sequencer = TestSequencer::<S> {
        user_info: additional_user,
        da_address: new_sequencer_address,
        bond: user_stake_value,
    };

    // We first bond the sequencer
    runner.execute(
        new_sequencer.create_plain_message::<RT, SequencerRegistry<S>>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: new_sequencer.da_address,
                amount: new_sequencer.bond,
            },
        ),
    );

    runner.config.sequencer_da_address = new_sequencer.da_address;

    runner
        // Then we use the non-default sequencer to set a value
        .execute_batch(BatchTestCase {
            input: vec![new_sequencer.create_plain_message::<RT, ValueSetter<S>>(
                sov_value_setter::CallMessage::SetValue {
                    value: 10,
                    gas: None,
                },
            )]
            .into(),
            assert: Box::new(move |result, state| {
                assert_eq!(result.sender_da_address, new_sequencer_address);
                // ensure the tx was applied / batch was accepted
                assert_eq!(
                    sov_value_setter::ValueSetter::<S>::default()
                        .value
                        .get(state)
                        .unwrap_infallible(),
                    Some(10)
                );
            }),
        });
}

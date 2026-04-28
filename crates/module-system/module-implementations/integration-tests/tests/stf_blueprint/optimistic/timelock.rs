use std::num::NonZeroU64;

use sov_modules_api::capabilities::TimelockPolicy;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_test_modules::hooks_count::TxHooksCount;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{
    generate_optimistic_runtime_with_kernel, AsUser, TestUser, TransactionTestCase,
};
use sov_value_setter::{CallMessage, ValueSetterConfig};

use crate::stf_blueprint::S;

generate_optimistic_runtime_with_kernel!(
    TimelockRuntime <=
    kernel_type: sov_test_utils::runtime::BasicKernel<'a, S>,
    modules: [
        value_setter: ValueSetter<S>,
        tx_hooks_count: TxHooksCount<S>,
        timelock: sov_timelock::Timelock<S>
    ],
    timelock_policy_wrapper: |call: &TimelockRuntimeCall<S>| {
        match call {
            TimelockRuntimeCall::ValueSetter(CallMessage::SetValue { value: 7, .. }) => {
                Some(TimelockPolicy {
                    unlock_seconds_from_proposal: NonZeroU64::new(60).unwrap(),
                    expire_seconds_after_unlock_override: Some(60),
                })
            }
            _ => None,
        }
    },
);

type RT = TimelockRuntime<S>;

fn setup() -> (TestUser<S>, TestRunner<RT, S>) {
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
        (),
        (),
    );

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    (admin, runner)
}

#[test]
fn timelocked_call_registers_proposal_without_dispatching_call() {
    let (admin, mut runner) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(CallMessage::SetValue {
            value: 7,
            gas: None,
        }),
        assert: Box::new(|result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(
                ValueSetter::<S>::default()
                    .value
                    .get(state)
                    .unwrap_infallible(),
                None
            );
            assert_eq!(
                TxHooksCount::<S>::default()
                    .post_dispatch_tx_hook_count
                    .get(state)
                    .unwrap_infallible(),
                Some(1)
            );
        }),
    });
}

#[test]
fn repeated_timelocked_call_still_registers_without_dispatching_call() {
    let (admin, mut runner) = setup();

    for count in 1..=2 {
        runner.execute_transaction(TransactionTestCase {
            input: admin.create_plain_message::<RT, ValueSetter<S>>(CallMessage::SetValue {
                value: 7,
                gas: None,
            }),
            assert: Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());
                assert_eq!(
                    ValueSetter::<S>::default()
                        .value
                        .get(state)
                        .unwrap_infallible(),
                    None
                );
                assert_eq!(
                    TxHooksCount::<S>::default()
                        .post_dispatch_tx_hook_count
                        .get(state)
                        .unwrap_infallible(),
                    Some(count)
                );
            }),
        });
    }
}

#[test]
fn untimelocked_call_dispatches_normally() {
    let (admin, mut runner) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(CallMessage::SetValue {
            value: 8,
            gas: None,
        }),
        assert: Box::new(|result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(
                ValueSetter::<S>::default()
                    .value
                    .get(state)
                    .unwrap_infallible(),
                Some(8)
            );
            assert_eq!(
                TxHooksCount::<S>::default()
                    .post_dispatch_tx_hook_count
                    .get(state)
                    .unwrap_infallible(),
                Some(1)
            );
        }),
    });
}

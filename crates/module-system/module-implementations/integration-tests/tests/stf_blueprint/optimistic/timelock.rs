use std::num::NonZeroU64;

use sov_modules_api::capabilities::{
    calculate_timelock_proposal_id, TimelockPolicy, TimelockProposalHashData,
    DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK,
};
use sov_modules_api::da::Time;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::EncodeCall;
use sov_test_modules::hooks_count::TxHooksCount;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{
    generate_optimistic_runtime_with_kernel, AsUser, TestUser, TransactionTestCase,
};
use sov_timelock::{CancellationPolicy, Timelock, TimelockKey};
use sov_value_setter::{CallMessage as ValueSetterCallMessage, ValueSetterConfig};

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
            TimelockRuntimeCall::ValueSetter(ValueSetterCallMessage::SetValue { value: 7, .. }) => {
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

    let mut runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
    runner.config.freeze_time = Some(Time::from_secs(1_000));

    (admin, runner)
}

fn set_value_message(value: u32) -> ValueSetterCallMessage<S> {
    ValueSetterCallMessage::SetValue { value, gas: None }
}

fn set_value_proposal_id(value: u32) -> sov_modules_api::capabilities::ProposalId {
    let encoded = <RT as EncodeCall<ValueSetter<S>>>::encode_call(set_value_message(value));
    calculate_timelock_proposal_id::<S>(TimelockProposalHashData::CallMessage(&encoded))
}

#[test]
fn timelocked_call_registers_proposal_without_dispatching_call() {
    let (admin, mut runner) = setup();
    let admin_address = admin.address();
    let proposal_id = set_value_proposal_id(7);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(set_value_message(7)),
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
                Some(1)
            );
            let condition = Timelock::<S>::default()
                .timelocks
                .get(
                    &TimelockKey {
                        address: admin_address,
                        proposal_id,
                    },
                    state,
                )
                .unwrap_infallible()
                .unwrap();
            assert_eq!(condition.executable_from, 1_060);
            assert_eq!(condition.executable_until, 1_120);
        }),
    });
}

#[test]
fn repeated_timelocked_call_reverts_while_locked() {
    let (admin, mut runner) = setup();
    let admin_address = admin.address();
    let proposal_id = set_value_proposal_id(7);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(set_value_message(7)),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(set_value_message(7)),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_reverted());
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
            assert!(Timelock::<S>::default()
                .timelocks
                .get(
                    &TimelockKey {
                        address: admin_address,
                        proposal_id,
                    },
                    state
                )
                .unwrap_infallible()
                .is_some());
        }),
    });
}

#[test]
fn unlocked_timelocked_call_dispatches_and_consumes_proposal() {
    let (admin, mut runner) = setup();
    let admin_address = admin.address();
    let proposal_id = set_value_proposal_id(7);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(set_value_message(7)),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    runner.config.freeze_time = Some(Time::from_secs(1_060));
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(set_value_message(7)),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(
                ValueSetter::<S>::default()
                    .value
                    .get(state)
                    .unwrap_infallible(),
                Some(7)
            );
            assert_eq!(
                TxHooksCount::<S>::default()
                    .post_dispatch_tx_hook_count
                    .get(state)
                    .unwrap_infallible(),
                Some(2)
            );
            assert!(Timelock::<S>::default()
                .timelocks
                .get(
                    &TimelockKey {
                        address: admin_address,
                        proposal_id,
                    },
                    state
                )
                .unwrap_infallible()
                .is_none());
        }),
    });
}

#[test]
fn owner_can_cancel_pending_proposal() {
    let (admin, mut runner) = setup();
    let admin_address = admin.address();
    let proposal_id = set_value_proposal_id(7);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(set_value_message(7)),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Timelock<S>>(
            sov_timelock::CallMessage::CancelUnlock {
                call_message_hash: proposal_id,
                address: None,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert!(Timelock::<S>::default()
                .timelocks
                .get(
                    &TimelockKey {
                        address: admin_address,
                        proposal_id,
                    },
                    state
                )
                .unwrap_infallible()
                .is_none());
        }),
    });
}

#[test]
fn cancellation_policy_updates_are_self_timelocked_after_initial_policy() {
    let (admin, mut runner) = setup();
    let admin_address = admin.address();
    let initial_policy = CancellationPolicy::<S> {
        authorized_canceller: admin_address,
        policy_change_timelock_seconds: 60,
    };
    let replacement_policy = CancellationPolicy::<S> {
        authorized_canceller: admin_address,
        policy_change_timelock_seconds: 120,
    };

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Timelock<S>>(
            sov_timelock::CallMessage::ModifyCancellationPolicy {
                new_policy: initial_policy.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Timelock<S>>(
            sov_timelock::CallMessage::ModifyCancellationPolicy {
                new_policy: replacement_policy.clone(),
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(
                Timelock::<S>::default()
                    .policies
                    .get(&admin_address, state)
                    .unwrap_infallible(),
                Some(initial_policy)
            );
        }),
    });

    runner.config.freeze_time = Some(Time::from_secs(1_060));
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Timelock<S>>(
            sov_timelock::CallMessage::ModifyCancellationPolicy {
                new_policy: replacement_policy.clone(),
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(
                Timelock::<S>::default()
                    .policies
                    .get(&admin_address, state)
                    .unwrap_infallible(),
                Some(replacement_policy)
            );
        }),
    });
}

#[test]
fn cancellation_policy_update_proposals_expire_after_default_window() {
    let (admin, mut runner) = setup();
    let admin_address = admin.address();
    let initial_policy = CancellationPolicy::<S> {
        authorized_canceller: admin_address,
        policy_change_timelock_seconds: 60,
    };
    let replacement_policy = CancellationPolicy::<S> {
        authorized_canceller: admin_address,
        policy_change_timelock_seconds: 120,
    };

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Timelock<S>>(
            sov_timelock::CallMessage::ModifyCancellationPolicy {
                new_policy: initial_policy.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Timelock<S>>(
            sov_timelock::CallMessage::ModifyCancellationPolicy {
                new_policy: replacement_policy.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    let expired_proposal_time =
        i64::try_from(1_060 + DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK + 1).unwrap();
    runner.config.freeze_time = Some(Time::from_secs(expired_proposal_time));
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Timelock<S>>(
            sov_timelock::CallMessage::ModifyCancellationPolicy {
                new_policy: replacement_policy,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_reverted());
            assert_eq!(
                Timelock::<S>::default()
                    .policies
                    .get(&admin_address, state)
                    .unwrap_infallible(),
                Some(initial_policy)
            );
        }),
    });
}

#[test]
fn untimelocked_call_dispatches_normally() {
    let (admin, mut runner) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(set_value_message(8)),
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

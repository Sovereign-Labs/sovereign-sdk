use std::num::NonZeroU64;

use sov_modules_api::capabilities::TimelockPolicy;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::TxEffect;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{
    generate_optimistic_runtime_with_kernel, AsUser, TestUser, TransactionTestCase,
};
use sov_value_setter::{CallMessage as ValueSetterCallMessage, ValueSetterConfig};

use crate::stf_blueprint::S;

generate_optimistic_runtime_with_kernel!(
    NoopTimelockRuntime <=
    kernel_type: sov_test_utils::runtime::BasicKernel<'a, S>,
    modules: [
        value_setter: ValueSetter<S>
    ],
    timelock_policy_wrapper: |call: &NoopTimelockRuntimeCall<S>| {
        match call {
            NoopTimelockRuntimeCall::ValueSetter(
                ValueSetterCallMessage::SetValue { value: 7, .. }
            ) => {
                Some(TimelockPolicy {
                    unlock_seconds_from_proposal: NonZeroU64::new(60).unwrap(),
                    expire_seconds_after_unlock_override: Some(60),
                })
            }
            _ => None,
        }
    },
);

type RT = NoopTimelockRuntime<S>;

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
    );

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    (admin, runner)
}

#[test]
fn timelocked_call_is_rejected_when_timelock_capability_is_unavailable() {
    let (admin, mut runner) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(ValueSetterCallMessage::SetValue {
            value: 7,
            gas: None,
        }),
        assert: Box::new(|result, state| {
            let TxEffect::Reverted(contents) = &result.tx_receipt else {
                panic!("expected reverted transaction, got {:?}", result.tx_receipt);
            };
            assert!(contents
                .reason
                .to_string()
                .contains("Timelocks are not available"));
            assert_eq!(
                ValueSetter::<S>::default()
                    .value
                    .get(state)
                    .unwrap_infallible(),
                None
            );
        }),
    });
}

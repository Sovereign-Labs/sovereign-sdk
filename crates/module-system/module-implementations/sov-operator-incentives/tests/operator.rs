mod helpers;

use helpers::setup;
use sov_operator_incentives::{CallMessage, OperatorIncentives};
use sov_test_utils::{
    AsUser, TestAddress, TestUser, TransactionTestCase, TEST_DEFAULT_USER_BALANCE,
};

use crate::helpers::{RT, S};

#[test]
fn update_reward_address() {
    let original_reward_user = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    let original_reward_address = original_reward_user.address();

    let mut runner = setup(original_reward_user.clone());
    let new_reward_address = TestAddress::new([22; 28]);

    // Update the reward address to a new one.
    runner.execute_transaction(TransactionTestCase {
        input: original_reward_user.create_plain_message::<RT, OperatorIncentives<S>>(
            CallMessage::UpdateRewardAddress { new_reward_address },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            let addr = OperatorIncentives::<S>::default()
                .reward_address
                .get(state)
                .unwrap()
                .unwrap();

            assert_eq!(addr, new_reward_address);
        }),
    });

    // Attempt to update the reward address to the old one, which should fail.
    runner.execute_transaction(TransactionTestCase {
        input: original_reward_user.create_plain_message::<RT, OperatorIncentives<S>>(
            CallMessage::UpdateRewardAddress {
                new_reward_address: original_reward_address,
            },
        ),
        assert: Box::new(move |result, state| {
            let addr = OperatorIncentives::<S>::default()
                .reward_address
                .get(state)
                .unwrap()
                .unwrap();

            // The tx failed becouse the sender (original_reward_user) is not correct.
            assert!(result.tx_receipt.is_reverted());
            assert_eq!(addr, new_reward_address);
        }),
    });
}

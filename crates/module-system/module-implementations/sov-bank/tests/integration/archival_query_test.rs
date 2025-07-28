use sov_bank::{Bank, Coins};
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::Amount;
use sov_test_utils::{AsUser, TestSpec};

use crate::helpers::*;

/// Tests the archival query functionality of the bank module by doing some transfers and querying the old balances afterwards.
#[test]
fn transfer_token_and_query_old_balances() {
    let (
        TestData {
            token_name,
            token_id,
            user_high_token_balance: sender,
            user_no_token_balance: receiver,
            ..
        },
        mut runner,
    ) = setup();

    let sender_initial_token_balance = sender.token_balance(&token_name).unwrap();

    const AMOUNT_PER_TRANSFER: Amount = Amount::new(10);

    for height in 1u64..10 {
        runner.execute(sender.create_plain_message::<RT, Bank<TestSpec>>(
            sov_bank::CallMessage::Transfer {
                to: receiver.address(),
                coins: Coins {
                    amount: AMOUNT_PER_TRANSFER,
                    token_id,
                },
            },
        ));

        // Test queries for latest height.
        runner.query_visible_state(|state| {
            assert_eq!(
                Bank::<TestSpec>::default()
                    .get_balance_of(&receiver.address(), token_id, state)
                    .unwrap_infallible(),
                Some(AMOUNT_PER_TRANSFER.checked_mul(height.into()).unwrap())
            );
        });

        for height_to_query in (0..=height).map(RollupHeight::new) {
            runner.query_visible_state(|state| {
                let archival_state = &mut state.get_archival_state(height_to_query).unwrap();

                // Sender query deducted at every height
                assert_eq!(
                    Bank::<TestSpec>::default()
                        .get_balance_of(&sender.address(), token_id, archival_state)
                        .unwrap_infallible(),
                    Some(
                        sender_initial_token_balance
                            .checked_sub(
                                AMOUNT_PER_TRANSFER
                                    .checked_mul(height_to_query.get().into())
                                    .unwrap()
                            )
                            .unwrap()
                    )
                );

                let expected_receiver_balance = if height_to_query == RollupHeight::GENESIS {
                    None
                } else {
                    Some(
                        AMOUNT_PER_TRANSFER
                            .checked_mul(height_to_query.get().into())
                            .unwrap(),
                    )
                };
                assert_eq!(
                    Bank::<TestSpec>::default()
                        .get_balance_of(&receiver.address(), token_id, archival_state)
                        .unwrap_infallible(),
                    expected_receiver_balance
                );
            });
        }
    }
}

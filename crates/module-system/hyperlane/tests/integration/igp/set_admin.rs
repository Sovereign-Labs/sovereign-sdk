use sov_hyperlane_integration::{
    InterchainGasPaymaster, InterchainGasPaymasterCallMessage, InterchainGasPaymasterEvent,
};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::runtime::{setup, TestRuntimeEvent, RT, S};

#[test]
fn set_admin_by_current_admin() {
    let (mut runner, _, _, relayer, _, user) = setup();

    let new_admin = user.address();
    let previous_admin = relayer.address();

    let igp = InterchainGasPaymaster::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, InterchainGasPaymaster<S>>(
            InterchainGasPaymasterCallMessage::SetAdmin { new_admin },
        ),
        assert: Box::new(move |result, state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetAdmin should succeed when called by current admin"
            );

            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::InterchainGasPaymaster(
                        InterchainGasPaymasterEvent::AdminSet {
                            previous_admin: prev,
                            new_admin: new,
                        }
                    ) if *prev == previous_admin && *new == new_admin
                )
            }));

            let stored_admin = igp
                .admin
                .get(state)
                .expect("admin value retrieved")
                .expect("admin is set");

            assert_eq!(stored_admin, new_admin);
        }),
    });
}

#[test]
fn set_admin_by_non_admin_fails() {
    let (mut runner, _, _, _, _, user) = setup();

    let new_admin = user.address();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, InterchainGasPaymaster<S>>(
            InterchainGasPaymasterCallMessage::SetAdmin { new_admin },
        ),
        assert: Box::new(move |result, _| {
            assert!(
                !result.tx_receipt.is_successful(),
                "SetAdmin should fail when called by non-admin"
            );
        }),
    });
}

use std::str::FromStr as _;

use sov_hyperlane_integration::{CallMessage, Ism, Recipient};
use sov_hyperlane_register_module::{
    CallMessage as RegistrationCallMessage, SolanaDeployment, SolanaRegistration,
};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Base58Address, CredentialId, HexString, SafeVec, Spec};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::setup::{
    make_invalid_message, make_valid_message, register_basic_warp_route, setup, Mailbox,
    SetupParams, TestRuntimeEvent, RT, S, SOLANA_PROGRAM_ID,
};

#[test]
fn test_user_is_registered_correctly() {
    let SetupParams {
        mut runner,
        admin,
        user,
        ..
    } = setup();
    let route_id = register_basic_warp_route(&mut runner, &admin);

    let payer = [1u8; 32];
    let payer_addr = <S as Spec>::Address::from(payer);
    let embedded = [2u8; 32];
    let credential = CredentialId::from(embedded);
    let body = [payer, embedded].concat();
    let valid_message = make_valid_message(0, route_id, HexString::new(body));
    let message = HexString::new(SafeVec::try_from(valid_message.encode().0).unwrap());

    runner.query_state(|state| {
        assert!(
            !sov_accounts::Accounts::<S>::default()
                .is_explicitly_authorized(&payer_addr, &credential, state)
                .unwrap_infallible(),
            "Embedded credential should not yet be authorized for the payer address"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            metadata: HexString::new(SafeVec::new()),
            message,
        }),
        assert: Box::new(move |result, _| {
            let receipt = &result.tx_receipt;
            assert!(
                result.tx_receipt.is_successful(),
                "Recipient was not registered successfully: {receipt:?}"
            );

            assert_eq!(
                result.events.last().unwrap(),
                &TestRuntimeEvent::SolanaRegister(
                    sov_hyperlane_register_module::Event::UserRegistered {
                        address: payer_addr,
                        credential_id: credential,
                    }
                )
            );
        }),
    });

    runner.query_state(|state| {
        assert!(
            sov_accounts::Accounts::<S>::default()
                .is_explicitly_authorized(&payer_addr, &credential, state)
                .unwrap_infallible(),
            "Embedded credential should be authorized for the payer address after registration"
        );
    });
}

#[test]
fn test_two_payers_registering_same_embedded_both_succeed() {
    let SetupParams {
        mut runner,
        admin,
        user,
        ..
    } = setup();
    let route_id = register_basic_warp_route(&mut runner, &admin);

    let payer_a = [1u8; 32];
    let payer_a_addr = <S as Spec>::Address::from(payer_a);
    let payer_b = [3u8; 32];
    let payer_b_addr = <S as Spec>::Address::from(payer_b);
    let embedded = [2u8; 32];
    let credential = CredentialId::from(embedded);

    // First registration: (payer_a, embedded).
    let body_a = [payer_a, embedded].concat();
    let msg_a = make_valid_message(0, route_id, HexString::new(body_a));
    let envelope_a = HexString::new(SafeVec::try_from(msg_a.encode().0).unwrap());
    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            metadata: HexString::new(SafeVec::new()),
            message: envelope_a,
        }),
        assert: Box::new(move |result, _| {
            let receipt = &result.tx_receipt;
            assert!(
                result.tx_receipt.is_successful(),
                "First registration should succeed: {receipt:?}"
            );
        }),
    });

    // Same embedded credential, different payer — many-to-many authorization is allowed.
    let body_b = [payer_b, embedded].concat();
    let msg_b = make_valid_message(1, route_id, HexString::new(body_b));
    let envelope_b = HexString::new(SafeVec::try_from(msg_b.encode().0).unwrap());
    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            metadata: HexString::new(SafeVec::new()),
            message: envelope_b,
        }),
        assert: Box::new(move |result, _| {
            let receipt = &result.tx_receipt;
            assert!(
                result.tx_receipt.is_successful(),
                "Second registration with same embedded but different payer should also succeed: {receipt:?}"
            );
        }),
    });

    runner.query_state(|state| {
        let accounts = sov_accounts::Accounts::<S>::default();
        assert!(
            accounts
                .is_explicitly_authorized(&payer_a_addr, &credential, state)
                .unwrap_infallible(),
            "First (payer_a, embedded) authorization should be recorded"
        );
        assert!(
            accounts
                .is_explicitly_authorized(&payer_b_addr, &credential, state)
                .unwrap_infallible(),
            "Second (payer_b, embedded) authorization should also be recorded"
        );
    });
}

#[test]
fn test_handler_passes_through_if_not_register_message() {
    let SetupParams {
        mut runner,
        admin,
        user,
        ..
    } = setup();
    let route_id = register_basic_warp_route(&mut runner, &admin);

    let payer = [1u8; 32];
    let embedded = [2u8; 32];
    let body = [payer, embedded].concat();
    let valid_message = make_invalid_message(0, route_id, HexString::new(body));
    let message = HexString::new(SafeVec::try_from(valid_message.encode().0).unwrap());
    // while the call fails, we just want to assert the handler is passed on to the warp route
    // if we get this error then we know it did because this error originates in the Warp module
    let expected_error = format!(
        "Remote router for route {route_id} and origin {} not found",
        valid_message.origin_domain
    );

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            metadata: HexString::new(SafeVec::new()),
            message,
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            sov_rollup_interface::stf::TxEffect::Reverted(contents) => {
                assert_eq!(contents.reason.to_string(), expected_error);
            }
            _ => panic!(
                "Message should have been passed to Warp route: {:?}",
                result.tx_receipt
            ),
        }),
    });
}

#[test]
fn test_call_errors_if_sender_is_not_admin() {
    let SetupParams {
        mut runner, user, ..
    } = setup();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, SolanaRegistration<S>>(
            RegistrationCallMessage::Update {
                admin: None,
                deployment: None,
                ism: None,
            },
        ),
        assert: Box::new(move |result, _| match result.tx_receipt {
            sov_rollup_interface::stf::TxEffect::Reverted(contents) => {
                assert_eq!(
                    &contents.reason.to_string(),
                    "Module can only be called by the admin"
                );
            }
            _ => panic!("Update call should have reverted: {:?}", result.tx_receipt),
        }),
    });
}

#[test]
fn test_update_call() {
    let SetupParams {
        mut runner,
        user,
        module_admin,
        ..
    } = setup();

    let deployment = SolanaDeployment {
        program_id: Base58Address::from_str(SOLANA_PROGRAM_ID).unwrap(),
        domain_id: 1337,
    };
    let updated_admin = user.address();
    runner.execute_transaction(TransactionTestCase {
        input: module_admin.create_plain_message::<RT, SolanaRegistration<S>>(
            RegistrationCallMessage::Update {
                admin: Some(updated_admin),
                deployment: Some(deployment.clone()),
                ism: Some(Ism::AlwaysTrust),
            },
        ),
        assert: Box::new(move |result, _| {
            assert_eq!(
                result.events.first().unwrap(),
                &TestRuntimeEvent::SolanaRegister(sov_hyperlane_register_module::Event::Updated {
                    admin: Some(updated_admin),
                    deployment: Some(deployment),
                    ism: Some(Ism::AlwaysTrust)
                })
            );
        }),
    });

    runner.query_state(|state| {
        let module = sov_hyperlane_register_module::SolanaRegistration::default();
        assert_eq!(module.admin(state).unwrap(), Some(updated_admin));
        assert_eq!(
            module.deployment(state).unwrap(),
            Some(SolanaDeployment {
                program_id: Base58Address::from_str(SOLANA_PROGRAM_ID).unwrap(),
                domain_id: 1337,
            })
        );
        assert_eq!(
            module.ism(&[0; 32].into(), state).unwrap(),
            Some(Ism::AlwaysTrust)
        );
    });
}

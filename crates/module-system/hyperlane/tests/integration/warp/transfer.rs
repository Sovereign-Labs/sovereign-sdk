use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use sov_bank::event::Event as BankEvent;
use sov_bank::{config_gas_token_id, Amount, Bank, CallMessage as BankCallMessage, TokenId};
use sov_hyperlane_integration::warp::{Admin, StoredTokenKind, TokenKind};
use sov_hyperlane_integration::{
    CallMessage, HyperlaneAddress, Ism, Mailbox, Message, Warp, WarpCallMessage, WarpEvent,
    MESSAGE_VERSION,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{HexHash, HexString, SafeVec, Spec, TxEffect, VersionReader};
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase};

use super::runtime::*;

#[allow(clippy::too_many_arguments)]
fn do_inbound_transfer_success(
    runner: &mut TestRunner<RT, S>,
    admin: &TestUser<S>,
    from_domain: u32,
    from_router_address: HexHash,
    warp_route_id: HexHash,
    to: HexHash,
    amount: Amount,
    token_id: TokenId,
) {
    do_inbound_transfer_success_with_scaled_amount(
        runner,
        admin,
        from_domain,
        from_router_address,
        warp_route_id,
        to,
        amount,
        encode_amount(amount),
        token_id,
    );
}

#[allow(clippy::too_many_arguments)]
fn do_inbound_transfer_success_with_scaled_amount(
    runner: &mut TestRunner<RT, S>,
    admin: &TestUser<S>,
    from_domain: u32,
    from_router_address: HexHash,
    warp_route_id: HexHash,
    to: HexHash,
    local_amount: Amount,
    remote_amount: HexHash,
    token_id: TokenId,
) {
    let message_body = {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&to.0);
        out.extend_from_slice(&remote_amount.0);
        out
    };
    let message = inbound_message(
        from_domain,
        from_router_address,
        warp_route_id,
        message_body,
    );

    let to_address = <<S as Spec>::Address>::from_sender(to).unwrap();
    let balance_before = runner.query_state(|state| {
        let bank = Bank::<S>::default();
        bank.get_balance_of(&to_address, token_id, state)
            .unwrap_infallible()
            .unwrap_or_default()
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S, Warp<S>>>(CallMessage::Process {
            metadata: HexString(vec![].try_into().unwrap()),
            message: HexString(message.encode().0.try_into().unwrap()),
        }),
        assert: Box::new(move |result, state| {
            assert!(
                result.tx_receipt.is_successful(),
                "Inbound transfer should be successful but failed with: {:?}",
                result.tx_receipt
            );
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::TokenTransferReceived {
                        route_id,
                        recipient,
                        amount,
                        from_domain,
                    }) if route_id == &warp_route_id && recipient == &to && amount == &local_amount  && *from_domain == CONFIGURED_DOMAIN
                )),
                "Token transferred event should be emitted"
            );

            let bank = Bank::<S>::default();
            let balance_after = bank.get_balance_of(&to_address, token_id, state).unwrap_infallible().unwrap_or_default();
            assert_eq!(
                balance_before.0 + local_amount.0, balance_after.0,
                "Balance should update correctly"
            );
        }),
    });
}

#[allow(clippy::too_many_arguments)]
fn do_inbound_transfer_failure(
    runner: &mut TestRunner<RT, S>,
    admin: &TestUser<S>,
    from_domain: u32,
    from_router_address: HexHash,
    warp_route_id: HexHash,
    to: HexHash,
    amount: Amount,
    expected_error: &'static str,
) {
    let message_body = Warp::<S>::pack_transfer_body(to, amount, &StoredTokenKind::Native).unwrap();
    let message = inbound_message(
        from_domain,
        from_router_address,
        warp_route_id,
        message_body,
    );

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S, Warp<S>>>(CallMessage::Process {
            metadata: HexString(vec![].try_into().unwrap()),
            message: HexString(message.encode().0.try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert!(
                    reason.reason.to_string().contains(expected_error),
                    "Revert reason should contain '{expected_error}' but reverted with: {}",
                    reason.reason
                );
            }
            _ => panic!("Transaction should be reverted"),
        }),
    });
}

fn inbound_message(
    from_domain: u32,
    from_router_address: HexHash,
    warp_route_id: HexHash,
    message_body: Vec<u8>,
) -> Message {
    static NONCE: AtomicU32 = AtomicU32::new(0);
    let domain: u32 = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    Message {
        version: MESSAGE_VERSION,
        nonce: NONCE.fetch_add(1, Ordering::Relaxed),
        origin_domain: from_domain,
        sender: from_router_address,
        dest_domain: domain,
        recipient: warp_route_id,
        body: message_body.into(),
    }
}

fn do_outbound_transfer(
    runner: &mut TestRunner<RT, S>,
    admin: &TestUser<S>,
    warp_route_id: HexHash,
    amount: Amount,
    relayer: <S as Spec>::Address,
    remote_amount: HexHash,
) {
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::TransferRemote {
            warp_route: warp_route_id,
            destination_domain: CONFIGURED_DOMAIN,
            recipient: CONFIGURED_REMOTE_ROUTER_ADDRESS,
            amount,
            relayer: Some(relayer),
            gas_payment_limit: Amount::MAX
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Outbound transfer should be successful but failed with: {:?}",
                result.tx_receipt
            );
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::TokenTransferredRemote {
                        route_id,
                        to_domain,
                        recipient,
                        amount,
                    }) if route_id == &warp_route_id && *to_domain == CONFIGURED_DOMAIN && recipient == &CONFIGURED_REMOTE_ROUTER_ADDRESS && amount == &remote_amount
                )),
                "Token transferred event should be emitted"
            );
        }),
    });
}

fn do_outbound_transfer_failure(
    runner: &mut TestRunner<RT, S>,
    admin: &TestUser<S>,
    warp_route_id: HexHash,
    amount: Amount,
    relayer: <S as Spec>::Address,
    expected_error: impl AsRef<str> + 'static,
) {
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::TransferRemote {
            warp_route: warp_route_id,
            destination_domain: CONFIGURED_DOMAIN,
            recipient: CONFIGURED_REMOTE_ROUTER_ADDRESS,
            amount,
            relayer: Some(relayer),
            gas_payment_limit: Amount::MAX,
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert!(
                    reason.reason.to_string().contains(expected_error.as_ref()),
                    "Revert reason should contain '{}' but reverted with: '{}'",
                    expected_error.as_ref(),
                    reason.reason
                );
            }
            _ => panic!("Transaction should be reverted"),
        }),
    });
}

#[test]
fn test_transfer_roundtrip() {
    let (mut runner, admin, other, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);
    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);

    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(100),
        relayer.address(),
        encode_amount(Amount(100)),
    );
    do_inbound_transfer_success(
        &mut runner,
        &admin,
        1,
        HexString([1; 32]),
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        config_gas_token_id(),
    );
}

#[test]
fn test_transfer_inbound_fails_various_edge_cases() {
    let (mut runner, admin, other, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);

    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(100),
        relayer.address(),
        encode_amount(Amount(100)),
    );
    do_inbound_transfer_failure(
        &mut runner,
        &admin,
        2, // Origin domain is not enrolled
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        "origin 2 not found",
    );
    do_inbound_transfer_failure(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        HexString([0; 32]), // Remote router is the wrong address
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        "Enrolled router does not match sender",
    );
    do_inbound_transfer_failure(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        HexString([0; 32]), // Warp route id is wrong
        other.address().to_sender(),
        Amount(100),
        "No dedicated or default ISM found for recipient",
    );
    do_inbound_transfer_failure(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(101), // Amount is more than the amount of locked tokens
        "Failed to transfer token",
    );

    // Check that the correct transfer still succeeds after all of these failures
    do_inbound_transfer_success(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        config_gas_token_id(),
    );
}

#[test]
fn test_transfer_remote_fails_if_not_enough_balance() {
    let (mut runner, admin, _, relayer) = setup();
    let full_balance = admin.available_gas_balance;

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::TransferRemote {
            warp_route: warp_route_id,
            destination_domain: CONFIGURED_DOMAIN,
            recipient: CONFIGURED_REMOTE_ROUTER_ADDRESS,
            amount: full_balance,
            relayer: Some(relayer.address()),
            gas_payment_limit: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => {
                    assert!(
                        reason.reason.to_string().contains("Failed to transfer"),
                        "Transaction should be reverted with the correct error but reverted with: {}",
                        reason.reason
                    );
                }
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_transfer_remote_fails_if_domain_not_enrolled() {
    let (mut runner, admin, _, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::TransferRemote {
            warp_route: warp_route_id,
            destination_domain: 2,
            recipient: CONFIGURED_REMOTE_ROUTER_ADDRESS,
            amount: Amount(100),
            relayer: Some(relayer.address()),
            gas_payment_limit: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => {
                    assert!(
                        reason.reason.to_string().contains("does not have remote router for domain 2"),
                        "Transaction should be reverted with the correct error but reverted with: {}",
                        reason.reason
                    );
                }
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_transfer_remote_fails_if_route_does_not_exist() {
    let (mut runner, admin, _, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let _warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::TransferRemote {
            warp_route: HexString([0; 32]),
            destination_domain: 2,
            recipient: CONFIGURED_REMOTE_ROUTER_ADDRESS,
            amount: Amount(100),
            relayer: Some(relayer.address()),
            gas_payment_limit: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => assert!(
                    reason.reason.to_string().contains("not found"),
                    "Transaction should be reverted with the correct error but reverted with: {}",
                    reason.reason
                ),
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

fn encode_amount(amount: Amount) -> HexHash {
    let mut encoded = [0u8; 32];
    encoded[16..].copy_from_slice(&amount.0.to_be_bytes());
    HexString(encoded)
}

#[test]
fn test_inbound_transfer_fails_if_ism_rejects() {
    let (mut runner, admin, other, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let warp_route_id = register_basic_warp_route_and_enroll_router_with_ism(
        &mut runner,
        &admin,
        Ism::TrustedRelayer {
            relayer: admin.address().to_sender(),
        },
    );
    // Do an outbound transfer to ensure we have enough balance
    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(100),
        relayer.address(),
        encode_amount(Amount(100)),
    );
    // Try the outbound tranfser without using the trusted relayer
    do_inbound_transfer_failure(
        &mut runner,
        &other, // wrong relayer
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        "is trusted to relay messages for this ISM",
    );

    // Try the outbound tranfser using the correct relayer and ensure it succeeds
    do_inbound_transfer_success(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        config_gas_token_id(),
    );
}

#[test]
fn test_inbound_transfer_after_ism_update() {
    let (mut runner, admin, other, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    // Register warp route with always trust ISM
    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);

    // Fund the warp route
    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(200),
        relayer.address(),
        encode_amount(Amount(200)),
    );

    // Inbound transfer should succeed
    do_inbound_transfer_success(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        config_gas_token_id(),
    );

    // Update the ISM to trust only admin as relayer .
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
            warp_route: warp_route_id,
            ism: Some(Ism::TrustedRelayer {
                relayer: admin.address().to_sender(),
            }),
            admin: None,
            inbound_transferrable_tokens_limit: None,
            inbound_limit_replenishment_per_slot: None,
            outbound_transferrable_tokens_limit: None,
            outbound_limit_replenishment_per_slot: None,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Route update should succeed"
            );
        }),
    });

    // Now inbound transfer submitted by untrusted relayer should fail
    do_inbound_transfer_failure(
        &mut runner,
        &relayer, // wrong relayer
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        admin.address().to_sender(),
        Amount(100),
        "is trusted to relay messages for this ISM",
    );

    // But the one submitted by trusted relayer should work
    do_inbound_transfer_success(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        config_gas_token_id(),
    );
}

#[test]
fn test_route_outbound_rate_limitting() {
    let (mut runner, admin, _other, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let warp_route_id = register_basic_warp_route_and_enroll_router_with_rate_limiter(
        &mut runner,
        &admin,
        Amount(10000),
        Amount(2000),
    );

    let assert_current_limit = |runner: &mut TestRunner<TestRuntime<S>, S>, current_limit| {
        runner.query_state(|state| {
            let warp = Warp::<S>::default();
            let route = warp
                .get_route(warp_route_id, state)
                .unwrap_infallible()
                .unwrap();

            let visible_slot = state.current_visible_slot_number();

            assert_eq!(
                route
                    .outbound_rate_limiter
                    .current_limit_with_replenishment(visible_slot),
                current_limit,
            );
        });
    };

    // Route is created in slot 2 and then in slot 3 the router is enrolled, which replenishes
    // limit once, and then second time during the transaction.
    assert_current_limit(&mut runner, Amount(4000));
    do_outbound_transfer_failure(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(6001),
        relayer.address(),
        "6001 exceeds current limit 6000",
    );

    // The slot advances again so the new limit is 8000.
    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(8000),
        relayer.address(),
        encode_amount(Amount(8000)),
    );

    // Now it should be completely drained.
    assert_current_limit(&mut runner, Amount(0));

    // In 5 slots, we should be back to maximum
    runner.advance_slots(5);
    assert_current_limit(&mut runner, Amount(10000));

    // and it shouldn't increase more.
    runner.advance_slots(1);
    assert_current_limit(&mut runner, Amount(10000));
    do_outbound_transfer_failure(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(10001),
        relayer.address(),
        "10001 exceeds current limit 10000",
    );
    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(10000),
        relayer.address(),
        encode_amount(Amount(10000)),
    );
}

#[test]
fn test_route_inbound_rate_limitting() {
    let (mut runner, admin, _other, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let warp_route_id = register_basic_warp_route_and_enroll_router_with_rate_limiter(
        &mut runner,
        &admin,
        Amount(10000),
        Amount(2000),
    );

    let assert_current_limit = |runner: &mut TestRunner<TestRuntime<S>, S>, current_limit| {
        runner.query_state(|state| {
            let warp = Warp::<S>::default();
            let route = warp
                .get_route(warp_route_id, state)
                .unwrap_infallible()
                .unwrap();

            let visible_slot = state.current_visible_slot_number();

            assert_eq!(
                route
                    .inbound_rate_limiter
                    .current_limit_with_replenishment(visible_slot),
                current_limit,
            );
        });
    };

    // Route is created in slot 2 and then in slot 3 the router is enrolled, which replenishes
    // limit once.
    assert_current_limit(&mut runner, Amount(4000));
    // Tokens need to be locked in a route for inbound transfers to succeed, so wait until we are
    // at full limit and feed the route.
    runner.advance_slots(3);
    assert_current_limit(&mut runner, Amount(10000));
    // Feed the route
    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(10000),
        relayer.address(),
        encode_amount(Amount(10000)),
    );
    // amount check runs for inbound transfers, which should be untouched
    assert_current_limit(&mut runner, Amount(10000));
    // feed the route once again
    runner.advance_slots(5);
    do_outbound_transfer(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(10000),
        relayer.address(),
        encode_amount(Amount(10000)),
    );

    // inbound limit should still be untouched
    assert_current_limit(&mut runner, Amount(10000));
    do_inbound_transfer_failure(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        relayer.address().to_sender(),
        Amount(10001),
        "10001 exceeds current limit 10000",
    );

    // Drain the route
    do_inbound_transfer_success(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        relayer.address().to_sender(),
        Amount(10000),
        config_gas_token_id(),
    );
    assert_current_limit(&mut runner, Amount(0));

    // We only replenished once, so this should fail
    do_inbound_transfer_failure(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        relayer.address().to_sender(),
        Amount(2001),
        "2001 exceeds current limit 2000",
    );

    // Now we replenished again, we can drain again
    do_inbound_transfer_success(
        &mut runner,
        &admin,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        relayer.address().to_sender(),
        Amount(4000),
        config_gas_token_id(),
    );
    assert_current_limit(&mut runner, Amount(0));
}

#[test]
fn test_collateral_route() {
    let (mut runner, admin, other, relayer, ..) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);
    let new_token_id = Arc::new(std::sync::Mutex::new(None));
    let token_id_ref = new_token_id.clone();
    runner.execute_transaction(TransactionTestCase {
        input: other.create_plain_message::<RT, Bank<S>>(BankCallMessage::CreateToken {
            token_name: "test".to_string().try_into().unwrap(),
            token_decimals: None,
            initial_balance: Amount(1000),
            mint_to_address: other.address(),
            admins: vec![other.address()].try_into().unwrap(),
            supply_cap: None,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Token should be created successfully"
            );
            let token_id = result.events.iter().find_map(|event| {
                if let TestRuntimeEvent::Bank(BankEvent::TokenCreated { coins, .. }) = event {
                    Some(coins.token_id)
                } else {
                    None
                }
            });
            *token_id_ref.lock().unwrap() = token_id;
        }),
    });

    let token_id = new_token_id.lock().unwrap().unwrap();
    let warp_route_id = register_warp_route_with_ism_and_token_source(
        &mut runner,
        &admin,
        Ism::AlwaysTrust,
        TokenKind::Collateral { token: token_id },
    );
    enroll_router(&mut runner, &admin, warp_route_id);
    // Outbond transfer from the admin should fail because of insufficient balance
    do_outbound_transfer_failure(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(100),
        relayer.address(),
        "Failed to transfer token",
    );
    // Inbound transfer should fail because the warp module has no tokens
    do_inbound_transfer_failure(
        &mut runner,
        &other,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        "Failed to transfer token",
    );
    // Outbound transfer from the other user should succeed, since we minted the token to him
    do_outbound_transfer(
        &mut runner,
        &other,
        warp_route_id,
        Amount(100),
        relayer.address(),
        encode_amount(Amount(100)),
    );
    // Inbound transfer should succeed now
    do_inbound_transfer_success(
        &mut runner,
        &other,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        Amount(100),
        token_id,
    );
}

fn register_synthetic_route(
    runner: &mut TestRunner<RT, S>,
    admin: &TestUser<S>,
    token_source: TokenKind,
    ism: Ism,
) -> (HexHash, TokenId) {
    // The borrow checker doesn't know that the closure runs before the end of execute transaction, so it complains about lifetimes
    // if we don't Arc the warp route id
    let warp_route_id = Arc::new(std::sync::Mutex::new(HexString([0; 32])));
    let warp_route_id_ref = warp_route_id.clone();
    let local_token_id = Arc::new(std::sync::Mutex::new(None));
    let local_token_id_ref = local_token_id.clone();
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Register {
            admin: Admin::InsecureOwner(admin.address()),
            token_source,
            ism,
            remote_routers: SafeVec::new(),
            inbound_transferrable_tokens_limit: Amount::MAX,
            inbound_limit_replenishment_per_slot: Amount::MAX,
            outbound_transferrable_tokens_limit: Amount::MAX,
            outbound_limit_replenishment_per_slot: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Recipient was not registered successfully"
            );
            for event in result.events {
                if let TestRuntimeEvent::Warp(WarpEvent::RouteRegistered {
                    route_id,
                    token_source,
                    ..
                }) = event
                {
                    *warp_route_id_ref.lock().unwrap() = route_id;
                    let StoredTokenKind::Synthetic { local_token_id, .. } = token_source else {
                        panic!("Token source should be synthetic");
                    };
                    *local_token_id_ref.lock().unwrap() = Some(local_token_id);
                }
            }
        }),
    });
    let local_token_id = local_token_id.lock().unwrap().unwrap();
    let route_id = *warp_route_id.lock().unwrap();
    assert!(
        route_id != HexString([0; 32]),
        "Warp route was not registered"
    );
    (route_id, local_token_id)
}

fn test_synthetic_route(
    token: TokenKind,
    amount_sent_inbound: Amount,
    amount_received_inbound: Amount,
    amount_sent_outbound: Amount,
    amount_received_outbound: Amount,
) {
    let (mut runner, admin, other, relayer) = setup();

    register_relayer_with_dummy_igp(&mut runner, &relayer, CONFIGURED_DOMAIN);

    let (warp_route_id, synthetic_token_id) =
        register_synthetic_route(&mut runner, &admin, token, Ism::AlwaysTrust);
    enroll_router(&mut runner, &admin, warp_route_id);
    // Outbond transfer from the admin should fail because of insufficient balance
    do_outbound_transfer_failure(
        &mut runner,
        &admin,
        warp_route_id,
        amount_received_inbound,
        relayer.address(),
        format!("supply=0 is less than burn amount={amount_received_inbound}"),
    );
    // Inbound transfer should succeed
    do_inbound_transfer_success_with_scaled_amount(
        &mut runner,
        &other,
        CONFIGURED_DOMAIN,
        CONFIGURED_REMOTE_ROUTER_ADDRESS,
        warp_route_id,
        other.address().to_sender(),
        amount_received_inbound,
        encode_amount(amount_sent_inbound),
        synthetic_token_id,
    );

    // Outbond transfer from the admin should fail because of insufficient balance
    do_outbound_transfer_failure(
        &mut runner,
        &admin,
        warp_route_id,
        Amount(1), // Amount is more than the amount of locked tokens
        relayer.address(),
        "Insufficient balance",
    );

    // Outbond transfer from the other user should fail because of insufficient balance
    let amount_bigger_than_balance = amount_received_inbound.checked_add(Amount(1)).unwrap();
    do_outbound_transfer_failure(
        &mut runner,
        &other,
        warp_route_id,
        amount_bigger_than_balance,
        relayer.address(),
        format!("supply={amount_received_inbound} is less than burn amount={amount_bigger_than_balance}"),
    );
    // Outbound transfer should succeed
    do_outbound_transfer(
        &mut runner,
        &other,
        warp_route_id,
        amount_sent_outbound,
        relayer.address(),
        encode_amount(amount_received_outbound),
    );
}

#[test]
fn test_synthetic_route_scaled_down() {
    test_synthetic_route(
        TokenKind::Synthetic {
            remote_token_id: HexString([255; 32]),
            remote_decimals: 18,
            local_decimals: Some(16),
        },
        Amount(10011), // send extra coins to test we round down correctly
        Amount(100),
        Amount(100),
        Amount(10000),
    );
}

#[test]
fn test_synthetic_route_scaled_up() {
    test_synthetic_route(
        TokenKind::Synthetic {
            remote_token_id: HexString([255; 32]),
            remote_decimals: 18,
            local_decimals: Some(20),
        },
        Amount(100),
        Amount(10000),
        Amount(9099),
        Amount(90),
    );
}

#[test]
fn test_synthetic_route_without_scaling() {
    test_synthetic_route(
        TokenKind::Synthetic {
            remote_token_id: HexString([255; 32]),
            remote_decimals: 18,
            local_decimals: None,
        },
        Amount(100),
        Amount(100),
        Amount(100),
        Amount(100),
    );
}

use serde::{Deserialize, Serialize};
use sov_bank::Amount;
use sov_hyperlane_integration::warp::{Admin, TokenKind};
use sov_hyperlane_integration::{HyperlaneAddress, Ism, Warp, WarpCallMessage, WarpEvent};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{HexHash, HexString, SafeVec, TxEffect, VersionReader};
use sov_test_utils::runtime::{ApiPath, TestRunner};
use sov_test_utils::{AsUser, TransactionTestCase};

use super::runtime::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteRouter {
    domain: u32,
    address: HexHash,
}

#[tokio::test(flavor = "multi_thread")]
async fn test_enroll_remote_router() {
    let (mut runner, admin, ..) = setup();
    let client = runner.setup_rest_api_server().await;

    let warp_route_id = register_basic_warp_route(&mut runner, &admin);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::EnrollRemoteRouter {
            warp_route: warp_route_id,
            remote_domain: CONFIGURED_DOMAIN,
            remote_router_address: CONFIGURED_REMOTE_ROUTER_ADDRESS,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Transaction should be successful"
            );
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouterEnrolled {
                        route_id,
                        domain,
                        router,
                    }) if route_id == &warp_route_id && *domain == 1 && router == &HexString([1; 32])
                )),
                "Router enrolled event should be emitted"
            );
        }),
    });

    let api_response = runner
        .query_api_response::<Vec<RemoteRouter>>(
            &ApiPath::query_module("warp")
                .with_custom_api_path(&format!("route/{warp_route_id}/routers")),
            &client,
        )
        .await;
    assert_eq!(api_response.len(), 1);
    assert_eq!(api_response[0].domain, CONFIGURED_DOMAIN);
    assert_eq!(api_response[0].address, CONFIGURED_REMOTE_ROUTER_ADDRESS);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_unenroll_remote_router() {
    let (mut runner, admin, ..) = setup();
    let client = runner.setup_rest_api_server().await;

    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);
    let api_response = runner
        .query_api_response::<Vec<RemoteRouter>>(
            &ApiPath::query_module("warp")
                .with_custom_api_path(&format!("route/{warp_route_id}/routers")),
            &client,
        )
        .await;
    assert_eq!(api_response.len(), 1);
    assert_eq!(api_response[0].domain, CONFIGURED_DOMAIN);
    assert_eq!(api_response[0].address, CONFIGURED_REMOTE_ROUTER_ADDRESS);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::UnEnrollRemoteRouter {
            warp_route: warp_route_id,
            remote_domain: CONFIGURED_DOMAIN,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Transaction should be successful"
            );
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouterUnEnrolled {
                        route_id,
                        domain,
                    }) if route_id == &warp_route_id && *domain == 1
                )),
                "Router unenrolled event should be emitted"
            );
        }),
    });
    let api_response = runner
        .query_api_response::<Vec<RemoteRouter>>(
            &ApiPath::query_module("warp")
                .with_custom_api_path(&format!("route/{warp_route_id}/routers")),
            &client,
        )
        .await;
    assert_eq!(api_response.len(), 0);
}

#[test]
fn test_unenroll_remote_router_fails_if_domain_not_enrolled() {
    let (mut runner, admin, ..) = setup();

    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::UnEnrollRemoteRouter {
            warp_route: warp_route_id,
            remote_domain: 2,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => assert!(
                    reason.reason.to_string().contains("not enrolled"),
                    "Transaction should be reverted with the correct error but reverted with: {}",
                    reason.reason
                ),
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_unenroll_remote_router_fails_if_route_does_not_exist() {
    let (mut runner, admin, ..) = setup();

    let _ = register_basic_warp_route_and_enroll_router(&mut runner, &admin);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::UnEnrollRemoteRouter {
            warp_route: HexString([0; 32]),
            remote_domain: CONFIGURED_DOMAIN,
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

#[test]
fn test_unenroll_remote_router_fails_if_not_admin() {
    let (mut runner, admin, other, ..) = setup();

    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);

    runner.execute_transaction(TransactionTestCase {
        input: other.create_plain_message::<RT, Warp<S>>(WarpCallMessage::UnEnrollRemoteRouter {
            warp_route: warp_route_id,
            remote_domain: CONFIGURED_DOMAIN,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => assert!(
                    reason
                        .reason
                        .to_string()
                        .contains("Cannot unenroll router with authorization from"),
                    "Transaction should be reverted with the correct error but reverted with: {}",
                    reason.reason
                ),
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_enroll_remote_router_fails_if_not_admin() {
    let (mut runner, admin, other, ..) = setup();

    let warp_route_id = register_basic_warp_route(&mut runner, &admin);
    runner.execute_transaction(TransactionTestCase {
        // Try to execute this transaction as the other user we registered, not the admin. This will reject.
        input: other.create_plain_message::<RT, Warp<S>>(WarpCallMessage::EnrollRemoteRouter {
            warp_route: warp_route_id,
            remote_domain: CONFIGURED_DOMAIN,
            remote_router_address: CONFIGURED_REMOTE_ROUTER_ADDRESS,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => assert!(
                    reason
                        .reason
                        .to_string()
                        .contains("Cannot enroll router with authorization from"),
                    "Transaction should be reverted with the correct error but reverted with: {}",
                    reason.reason
                ),
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_enroll_remote_router_fails_if_duplicate() {
    let (mut runner, admin, ..) = setup();

    let warp_route_id = register_basic_warp_route_and_enroll_router(&mut runner, &admin);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::EnrollRemoteRouter {
            // Try to execute this transaction as the other user we registered, not the admin. This will reject.
            warp_route: warp_route_id,
            remote_domain: CONFIGURED_DOMAIN,
            remote_router_address: CONFIGURED_REMOTE_ROUTER_ADDRESS,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => assert!(
                    reason.reason.to_string().contains("already enrolled"),
                    "Transaction should be reverted with the correct error but reverted with: {}",
                    reason.reason
                ),
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_register_warp_route_duplicate_registrations_fail() {
    let (mut runner, admin, ..) = setup();

    let _warp_route_id = register_basic_warp_route(&mut runner, &admin);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Register {
            admin: Admin::InsecureOwner(admin.address()),
            token_source: TokenKind::Native,
            ism: Ism::AlwaysTrust,
            remote_routers: SafeVec::new(),
            inbound_transferrable_tokens_limit: Amount::MAX,
            inbound_limit_replenishment_per_slot: Amount::MAX,
            outbound_transferrable_tokens_limit: Amount::MAX,
            outbound_limit_replenishment_per_slot: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => assert!(
                    reason
                        .reason
                        .to_string()
                        .contains("already registered by sender"),
                    "Transaction should be reverted with the correct error but reverted with: {}",
                    reason.reason
                ),
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_enroll_remote_routers_on_registration() {
    let (mut runner, admin, ..) = setup();

    let routers = [(1, [1; 32].into()), (2, [2; 32].into())];
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Register {
            token_source: TokenKind::Native,
            admin: Admin::None,
            ism: Ism::AlwaysTrust,
            remote_routers: routers.as_ref().try_into().unwrap(),
            inbound_transferrable_tokens_limit: Amount::MAX,
            inbound_limit_replenishment_per_slot: Amount::MAX,
            outbound_transferrable_tokens_limit: Amount::MAX,
            outbound_limit_replenishment_per_slot: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Transaction should be successful"
            );
            let warp_route_id = result
                .events
                .iter()
                .find_map(|event| {
                    if let TestRuntimeEvent::Warp(WarpEvent::RouteRegistered { route_id, .. }) =
                        event
                    {
                        Some(route_id)
                    } else {
                        None
                    }
                })
                .expect("Route registered event should be emitted");

            for (expected_domain, expected_router) in routers {
                assert!(
                    result.events.iter().any(|event| matches!(
                        event,
                        TestRuntimeEvent::Warp(WarpEvent::RouterEnrolled { route_id, domain, router })
                        if route_id == warp_route_id
                            && domain == &expected_domain
                            && router == &expected_router
                    )),
                    "Router enrolled event for domain {expected_domain} should be emitted"
                );
            }
        }),
    });
}

#[test]
fn test_enroll_remote_routers_on_registration_fails_on_duplicates() {
    let (mut runner, admin, ..) = setup();

    let routers = [(1, [1; 32].into()), (1, [2; 32].into())];
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Register {
            token_source: TokenKind::Native,
            admin: Admin::None,
            ism: Ism::AlwaysTrust,
            remote_routers: routers.as_ref().try_into().unwrap(),
            inbound_transferrable_tokens_limit: Amount::MAX,
            inbound_limit_replenishment_per_slot: Amount::MAX,
            outbound_transferrable_tokens_limit: Amount::MAX,
            outbound_limit_replenishment_per_slot: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            match result.tx_receipt {
                TxEffect::Reverted(reason) => assert!(
                    reason.reason.to_string().contains("already enrolled"),
                    "Transaction should be reverted with the correct error but reverted with: {}",
                    reason.reason
                ),
                _ => panic!("Transaction should be reverted"),
            };
        }),
    });
}

#[test]
fn test_warp_route_updates() {
    let (mut runner, admin, relayer, ..) = setup();

    let warp_route_id = register_basic_warp_route(&mut runner, &admin);

    // Empty update
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
            warp_route: warp_route_id,
            admin: None,
            ism: None,
            inbound_transferrable_tokens_limit: None,
            inbound_limit_replenishment_per_slot: None,
            outbound_transferrable_tokens_limit: None,
            outbound_limit_replenishment_per_slot: None,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_reverted(),
                "Empty update should be reverted"
            );
        }),
    });

    // Update the admin
    let relayer_addr = relayer.address();
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
            warp_route: warp_route_id,
            admin: Some(Admin::InsecureOwner(relayer_addr)),
            ism: None,
            inbound_transferrable_tokens_limit: None,
            inbound_limit_replenishment_per_slot: None,
            outbound_transferrable_tokens_limit: None,
            outbound_limit_replenishment_per_slot: None,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteUpdated {
                        route_id,
                        updated_admin,
                        updated_ism,
                    }) if route_id == &warp_route_id
                        && updated_admin == &Some(Admin::InsecureOwner(relayer_addr))
                        && updated_ism.is_none()
                )),
                "Route updated event should be emitted"
            );
        }),
    });

    // Update the ism
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
            warp_route: warp_route_id,
            admin: None,
            ism: Some(Ism::TrustedRelayer {
                relayer: relayer_addr.to_sender(),
            }),
            inbound_transferrable_tokens_limit: None,
            inbound_limit_replenishment_per_slot: None,
            outbound_transferrable_tokens_limit: None,
            outbound_limit_replenishment_per_slot: None,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteUpdated {
                        route_id,
                        updated_admin,
                        updated_ism,
                    }) if route_id == &warp_route_id
                        && updated_admin.is_none()
                        && updated_ism == &Some(Ism::TrustedRelayer { relayer: relayer_addr.to_sender() })
                )),
                "Route updated event should be emitted"
            );
        }),
    });

    // Update admin and ism and limits
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
            warp_route: warp_route_id,
            admin: Some(Admin::None),
            ism: Some(Ism::AlwaysTrust),
            inbound_transferrable_tokens_limit: Some(Amount(1234)),
            inbound_limit_replenishment_per_slot: Some(Amount(4321)),
            outbound_transferrable_tokens_limit: Some(Amount(1234)),
            outbound_limit_replenishment_per_slot: Some(Amount(4321)),
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteUpdated {
                        route_id,
                        updated_admin,
                        updated_ism,
                    }) if route_id == &warp_route_id
                        && updated_admin == &Some(Admin::None)
                        && updated_ism == &Some(Ism::AlwaysTrust)
                )),
                "Route updated event should be emitted"
            );
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteInboundLimitsUpdated {
                        route_id,
                        updated_transferrable_tokens_limit,
                        updated_limit_replenishment_per_slot,
                    }) if route_id == &warp_route_id
                        && updated_transferrable_tokens_limit == &Some(Amount(1234))
                        && updated_limit_replenishment_per_slot == &Some(Amount(4321))
                )),
                "Route inbound limits updated event should be emitted"
            );
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteOutboundLimitsUpdated {
                        route_id,
                        updated_transferrable_tokens_limit,
                        updated_limit_replenishment_per_slot,
                    }) if route_id == &warp_route_id
                        && updated_transferrable_tokens_limit == &Some(Amount(1234))
                        && updated_limit_replenishment_per_slot == &Some(Amount(4321))
                )),
                "Route outbound limits updated event should be emitted"
            );
        }),
    });

    // After setting admin to None, no one should be able to further update route
    for owner in [admin, relayer] {
        runner.execute_transaction(TransactionTestCase {
            input: owner.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
                warp_route: warp_route_id,
                admin: Some(Admin::InsecureOwner(owner.address())),
                ism: None,
                inbound_transferrable_tokens_limit: None,
                inbound_limit_replenishment_per_slot: None,
                outbound_transferrable_tokens_limit: None,
                outbound_limit_replenishment_per_slot: None,
            }),
            assert: Box::new(move |result, _| {
                assert!(
                    result.tx_receipt.is_reverted(),
                    "Route update should be revereted because ownership was dropped"
                );
            }),
        });
    }
}

#[test]
fn test_warp_route_independent_limits() {
    let (mut runner, admin, ..) = setup();

    let warp_route_id = register_basic_warp_route(&mut runner, &admin);

    // update only inbound limits
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
            warp_route: warp_route_id,
            admin: None,
            ism: None,
            inbound_transferrable_tokens_limit: Some(Amount(1234)),
            inbound_limit_replenishment_per_slot: Some(Amount(4321)),
            outbound_transferrable_tokens_limit: None,
            outbound_limit_replenishment_per_slot: None,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteInboundLimitsUpdated {
                        route_id,
                        updated_transferrable_tokens_limit,
                        updated_limit_replenishment_per_slot,
                    }) if route_id == &warp_route_id
                        && updated_transferrable_tokens_limit == &Some(Amount(1234))
                        && updated_limit_replenishment_per_slot == &Some(Amount(4321))
                )),
                "Route inbound limits updated event should be emitted"
            );
            assert!(
                !result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteOutboundLimitsUpdated { .. })
                )),
                "Route outbound limits shouldn't be updated"
            );
        }),
    });

    // update only outbound limits
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
            warp_route: warp_route_id,
            admin: None,
            ism: None,
            inbound_transferrable_tokens_limit: None,
            inbound_limit_replenishment_per_slot: None,
            outbound_transferrable_tokens_limit: Some(Amount(1234)),
            outbound_limit_replenishment_per_slot: Some(Amount(4321)),
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteOutboundLimitsUpdated {
                        route_id,
                        updated_transferrable_tokens_limit,
                        updated_limit_replenishment_per_slot,
                    }) if route_id == &warp_route_id
                        && updated_transferrable_tokens_limit == &Some(Amount(1234))
                        && updated_limit_replenishment_per_slot == &Some(Amount(4321))
                )),
                "Route outbound limits updated event should be emitted"
            );
            assert!(
                !result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::Warp(WarpEvent::RouteInboundLimitsUpdated { .. })
                )),
                "Route inbound limits shouldn't be updated"
            );
        }),
    });
}

#[test]
fn test_warp_route_limit_updates() {
    let (mut runner, admin, ..) = setup();

    let warp_route_id = register_basic_warp_route(&mut runner, &admin);

    let update_limits = |runner: &mut TestRunner<TestRuntime<S>, S>, max, replenishment| {
        runner.execute_transaction(TransactionTestCase {
            input: admin.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Update {
                warp_route: warp_route_id,
                admin: None,
                ism: None,
                inbound_transferrable_tokens_limit: max,
                inbound_limit_replenishment_per_slot: replenishment,
                outbound_transferrable_tokens_limit: max,
                outbound_limit_replenishment_per_slot: replenishment,
            }),
            assert: Box::new(move |result, _| {
                assert!(
                    result.events.iter().any(|event| matches!(
                        event,
                        TestRuntimeEvent::Warp(WarpEvent::RouteInboundLimitsUpdated {
                            route_id,
                            updated_transferrable_tokens_limit,
                            updated_limit_replenishment_per_slot,
                        }) if route_id == &warp_route_id
                            && updated_transferrable_tokens_limit == &max
                            && updated_limit_replenishment_per_slot == &replenishment
                    )),
                    "Route inbound limits updated event should be emitted"
                );
                assert!(
                    result.events.iter().any(|event| matches!(
                        event,
                        TestRuntimeEvent::Warp(WarpEvent::RouteOutboundLimitsUpdated {
                            route_id,
                            updated_transferrable_tokens_limit,
                            updated_limit_replenishment_per_slot,
                        }) if route_id == &warp_route_id
                            && updated_transferrable_tokens_limit == &max
                            && updated_limit_replenishment_per_slot == &replenishment
                    )),
                    "Route outbound limits updated event should be emitted"
                );
            }),
        });
    };
    let assert_max_and_current_limits =
        |runner: &mut TestRunner<TestRuntime<S>, S>, max, current| {
            runner.query_state(|state| {
                let warp = Warp::<S>::default();
                let route = warp
                    .get_route(warp_route_id, state)
                    .unwrap_infallible()
                    .unwrap();

                let visible_slot = state.current_visible_slot_number();

                assert_eq!(route.outbound_rate_limiter.max_limit(), max);
                assert_eq!(
                    route
                        .outbound_rate_limiter
                        .current_limit_with_replenishment(visible_slot),
                    current,
                );
            });
        };

    // route was enrolled ignoring rate limits
    assert_max_and_current_limits(&mut runner, Amount::MAX, Amount::MAX);

    // if current limit would be bigger than max limit, it should be truncated after update
    // note that we also lowered replenishment to 1000 tokens per slot
    update_limits(&mut runner, Some(Amount(10000)), Some(Amount(1000)));
    assert_max_and_current_limits(&mut runner, Amount(10000), Amount(10000));

    // but when raising the maximal limit, the current limit shouldn't change
    update_limits(&mut runner, Some(Amount(100000)), None);
    // note that with slot advancement we replenished some
    assert_max_and_current_limits(&mut runner, Amount(100000), Amount(11000));

    // current shouldn't change as well if we lower the maximal limit to a value bigger than it
    update_limits(&mut runner, Some(Amount(50000)), None);
    // note that with slot advancement we replenished some
    assert_max_and_current_limits(&mut runner, Amount(50000), Amount(12000));

    // now we raise replenishment per slot, but it should still replenish at the beginning of this
    // slot once, based on the old rules
    update_limits(&mut runner, None, Some(Amount(10000)));
    assert_max_and_current_limits(&mut runner, Amount(50000), Amount(13000));

    // in new slot, the replenishment update should be reflected
    runner.advance_slots(1);
    assert_max_and_current_limits(&mut runner, Amount(50000), Amount(23000));
}

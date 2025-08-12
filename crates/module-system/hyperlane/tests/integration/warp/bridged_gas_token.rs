use sov_hyperlane_integration::warp::{Admin, StoredTokenKind, TokenKind, WarpRouteId};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::{PriorityFeeBips, TxDetails};
use sov_test_utils::{
    sov_sequencer_registry, AsUser, TestSequencer, TransactionTestCase, TransactionType,
};
use std::collections::HashMap;
use std::sync::Arc;

use sov_bank::{Amount, Bank, TokenId};
use sov_hyperlane_integration::igp::ExchangeRateAndGasPrice;
use sov_hyperlane_integration::{
    CallMessage as MailboxCallMessage, HyperlaneAddress, InterchainGasPaymasterCallMessage, Ism,
    WarpCallMessage, WarpEvent,
};
use sov_modules_api::{HexHash, HexString, SafeVec};
use sov_modules_api::{SafeString, Spec, TxEffect};
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::sov_chain_state;
use sov_test_utils::TestUser;
const TEST_SETUP_MODE_DISABLE_AT_SLOT: u64 = 10;

use crate::igp::{default_gas_hashmap_to_safe_vec, oracle_data_hashmap_to_safe_vec};
use crate::warp::transfer::{encode_amount, inbound_message};

use super::runtime::*;

#[allow(clippy::type_complexity)]
pub fn setup_without_gas_token() -> (
    TestRunner<TestRuntime<S>, S>,
    TestUser<S>,
    TestUser<S>,
    TestUser<S>,
    TestSequencer<S>,
) {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_SETUP_MODE_TERMINATION_HEIGHT",
        TEST_SETUP_MODE_DISABLE_AT_SLOT.to_string(),
    );
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(3);
    let preferred_sequencer = genesis_config.initial_sequencer.clone();

    let admin_account = genesis_config.additional_accounts()[0].clone();
    let extra_account = genesis_config.additional_accounts()[1].clone();
    let relayer_account = genesis_config.additional_accounts()[1].clone();

    let mut rt_genesis_config =
        GenesisConfig::from_minimal_config(genesis_config.clone().into(), (), (), (), ());

    rt_genesis_config.chain_state.admin = Some(admin_account.address());
    rt_genesis_config.bank.gas_token_config = None;

    rt_genesis_config
        .sequencer_registry
        .sequencer_config
        .seq_bond = Amount::ZERO;
    rt_genesis_config
        .attester_incentives
        .initial_attesters
        .iter_mut()
        .for_each(|(_addr, bond)| {
            *bond = Amount::ZERO;
        });
    rt_genesis_config
        .prover_incentives
        .initial_provers
        .iter_mut()
        .for_each(|(_addr, bond)| {
            *bond = Amount::ZERO;
        });

    (
        TestRunner::new_with_genesis(rt_genesis_config.into_genesis_params(), Default::default()),
        admin_account,
        extra_account,
        relayer_account,
        preferred_sequencer,
    )
}

fn register_relayer_gasless(runner: &mut TestRunner<RT, S>, relayer: &TestUser<S>, domain: u32) {
    let domain_oracle_data = HashMap::from([(
        domain,
        ExchangeRateAndGasPrice {
            gas_price: Amount(100),
            token_exchange_rate: 100,
        },
    )]);
    let domain_default_gas = HashMap::from([(domain, Amount(100))]);
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::Plain {
            message: TestRuntimeCall::InterchainGasPaymaster(
                InterchainGasPaymasterCallMessage::SetRelayerConfig {
                    domain_oracle_data: oracle_data_hashmap_to_safe_vec(domain_oracle_data.clone()),
                    domain_default_gas: default_gas_hashmap_to_safe_vec(domain_default_gas.clone()),
                    default_gas: Amount(100),
                    beneficiary: None,
                },
            ),
            key: relayer.private_key.clone(),
            details: TxDetails {
                max_fee: Amount::ZERO,
                max_priority_fee_bips: PriorityFeeBips::ZERO,
                gas_limit: None,
                chain_id: config_value!("CHAIN_ID"),
            },
        },
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "IGP set relayer config was not done successfully. {:?}",
                result.tx_receipt
            );
        }),
    });
}

pub fn enroll_router_gasless(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    warp_route_id: WarpRouteId,
) {
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::Plain {
            message: TestRuntimeCall::Warp(WarpCallMessage::EnrollRemoteRouter {
                warp_route: warp_route_id,
                remote_domain: CONFIGURED_DOMAIN,
                remote_router_address: CONFIGURED_REMOTE_ROUTER_ADDRESS,
            }),
            key: user.private_key.clone(),
            details: TxDetails {
                max_fee: Amount::ZERO,
                max_priority_fee_bips: PriorityFeeBips::ZERO,
                gas_limit: None,
                chain_id: config_value!("CHAIN_ID"),
            },
        },
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Enrollment transaction should be successful"
            );
        }),
    });
}

pub fn register_warp_route_gasless(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
) -> (WarpRouteId, TokenId) {
    // The borrow checker doesn't know that the closure runs before the end of execute transaction, so it complains about lifetimes
    // if we don't Arc the warp route id
    let warp_route_id = Arc::new(std::sync::Mutex::new(HexString([0; 32])));
    let id_ref = warp_route_id.clone();
    let token_id = Arc::new(std::sync::Mutex::new(TokenId::from([1; 32])));
    let token_id_ref = token_id.clone();
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::Plain {
            message: TestRuntimeCall::Warp(WarpCallMessage::Register {
                admin: Admin::InsecureOwner(user.address()),
                token_source: TokenKind::Synthetic {
                    remote_token_id: [1; 32].into(),
                    remote_decimals: 9,
                    local_decimals: None,
                },
                ism: Ism::AlwaysTrust,
                remote_routers: SafeVec::new(),
                inbound_transferrable_tokens_limit: Amount::MAX,
                inbound_limit_replenishment_per_slot: Amount::MAX,
                outbound_transferrable_tokens_limit: Amount::MAX,
                outbound_limit_replenishment_per_slot: Amount::MAX,
            }),
            key: user.private_key.clone(),
            details: TxDetails {
                max_fee: Amount::ZERO,
                max_priority_fee_bips: PriorityFeeBips::ZERO,
                gas_limit: None,
                chain_id: config_value!("CHAIN_ID"),
            },
        },
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
                    *id_ref.lock().unwrap() = route_id;
                    if let StoredTokenKind::Synthetic { local_token_id, .. } = token_source {
                        *token_id_ref.lock().unwrap() = local_token_id;
                    } else {
                        panic!("Token source was set to be synthetic");
                    }
                }
            }
        }),
    });
    let id = *warp_route_id.lock().unwrap();
    let token_id = *token_id.lock().unwrap();
    assert_ne!(id, HexString([0; 32]), "Warp route was not registered");
    assert_ne!(
        token_id,
        TokenId::from([1; 32]),
        "Token id was not set correctly"
    );
    (id, token_id)
}

/// Test setting up a token
#[allow(clippy::too_many_arguments)]
fn do_inbound_transfer_gasless(
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
        input: TransactionType::Plain {
			message: TestRuntimeCall::Mailbox(MailboxCallMessage::Process {
                metadata: HexString(vec![].try_into().unwrap()),
                message: HexString(message.encode().0.try_into().unwrap()),
            }),
			key: admin.private_key.clone(),
			details: TxDetails {
				max_fee: Amount::ZERO,
				max_priority_fee_bips: PriorityFeeBips::ZERO,
				gas_limit: None,
				chain_id: config_value!("CHAIN_ID"),
			}
        },
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

/// Test running the rollup with a bridged gas token
#[test]
fn test_bridged_gas_token() {
    // Start the rollup in admin mode (no gas fees)
    let (mut runner, admin, other, relayer, preferred_sequencer) = setup_without_gas_token();

    // Register the relayer and the warp route
    register_relayer_gasless(&mut runner, &relayer, CONFIGURED_DOMAIN);
    let (warp_route_id, token_id) = register_warp_route_gasless(&mut runner, &admin);
    enroll_router_gasless(&mut runner, &admin, warp_route_id);

    // Set the token ID to the newly created token
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_GAS_TOKEN_ID", token_id.to_string());

    // Do an inbound transfer to the "other" address
    do_inbound_transfer_gasless(
        &mut runner,
        &admin,
        1,
        HexString([1; 32]),
        warp_route_id,
        other.address().to_sender(),
        Amount(100_000_000_000_000),
        encode_amount(Amount(100_000_000_000_000)),
        token_id,
    );

    // Do an inbound transfer to the preferred sequencer address so that it has balance to bond
    do_inbound_transfer_gasless(
        &mut runner,
        &admin,
        1,
        HexString([1; 32]),
        warp_route_id,
        preferred_sequencer.user_info.address().to_sender(),
        Amount(100_000_000_000_000),
        encode_amount(Amount(100_000_000_000_000)),
        token_id,
    );

    // Bond the preferred sequencer before disabling admin mode
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::Plain {
            message: TestRuntimeCall::SequencerRegistry(
                sov_sequencer_registry::CallMessage::Deposit {
                    da_address: preferred_sequencer.da_address,
                    amount: Amount(1_000_000_000_000),
                },
            ),
            key: preferred_sequencer.user_info.private_key.clone(),
            details: TxDetails {
                max_fee: Amount::ZERO,
                max_priority_fee_bips: PriorityFeeBips::ZERO,
                gas_limit: None,
                chain_id: config_value!("CHAIN_ID"),
            },
        },
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Sequencer registry deposit should be successful"
            );
        }),
    });

    // Disable admin mode
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::Plain {
            message: TestRuntimeCall::ChainState(sov_chain_state::CallMessage::TerminateSetupMode),
            key: admin.private_key.clone(),
            details: TxDetails {
                max_fee: Amount::ZERO,
                max_priority_fee_bips: PriorityFeeBips::ZERO,
                gas_limit: None,
                chain_id: config_value!("CHAIN_ID"),
            },
        },
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Setup mode termination should be successful"
            );
        }),
    });

    // Check that the "other" user can successfully create a token (because it has gas)
    runner.execute_transaction(TransactionTestCase {
        input: other.create_plain_message::<_, sov_bank::Bank<S>>(
            sov_bank::CallMessage::CreateToken {
                token_name: SafeString::new(),
                token_decimals: None,
                initial_balance: Amount::ZERO,
                mint_to_address: admin.address(),
                admins: SafeVec::new(),
                supply_cap: None,
            },
        ),

        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Token creation should be successful"
            );
        }),
    });

    // Since we didn't send any funds to the admin, it can't transact
    runner.execute_transaction(TransactionTestCase {
		input: admin.create_plain_message::<_, sov_bank::Bank<S>>(
				sov_bank::CallMessage::CreateToken { token_name: SafeString::new(), token_decimals: None, initial_balance: Amount::ZERO, mint_to_address: admin.address(), admins: SafeVec::new(), supply_cap: None },
			),

		assert: Box::new(move |result, _| {
			let TxEffect::Skipped(reason) = result.tx_receipt else {
				panic!("Token creation should be skipped due to insufficient balance. Got receipt: {:?}", result.tx_receipt);
			};
			assert!(
				reason.error.to_string().contains("Insufficient balance"),
				"Token creation should be skipped due to insufficient balance. Got reason: {}",
				reason.error
			);
		}),
	});
}

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use sov_bank::{config_gas_token_id, Amount, Bank, IntoPayable};
use sov_hyperlane_integration::igp::ExchangeRateAndGasPrice;
use sov_hyperlane_integration::{
    CallMessage, Event as MailboxEvent, HyperlaneAddress, InterchainGasPaymaster,
    InterchainGasPaymasterCallMessage, InterchainGasPaymasterEvent, Message, MESSAGE_VERSION,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::{HexHash, HexString, Spec, TxEffect};
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::sov_paymaster::SafeVec;
use sov_test_utils::{AsUser, TransactionTestCase};

use super::{default_gas_hashmap_to_safe_vec, oracle_data_hashmap_to_safe_vec, IGPMetadata};
use crate::runtime::{
    register_recipient, setup, unlimited_gas_meter, Mailbox, TestRuntime, TestRuntimeEvent, RT, S,
};

const TOKEN_EXCHANGE_RATE_SCALE: u128 = 10u128.pow(19);

pub fn set_relayer_config<U: AsUser<S>>(
    runner: &mut TestRunner<TestRuntime<S>, S>,
    relayer: &U,
    domain_oracle_data: HashMap<u32, ExchangeRateAndGasPrice>,
    domain_default_gas: HashMap<u32, Amount>,
    default_gas: Amount,
    beneficiary: Option<<S as Spec>::Address>,
) {
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, InterchainGasPaymaster<S>>(
            InterchainGasPaymasterCallMessage::SetRelayerConfig {
                domain_oracle_data: oracle_data_hashmap_to_safe_vec(domain_oracle_data),
                domain_default_gas: default_gas_hashmap_to_safe_vec(domain_default_gas),
                default_gas,
                beneficiary,
            },
        ),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::InterchainGasPaymaster(
                        InterchainGasPaymasterEvent::RelayerConfigSet { .. }
                    )
                )
            }));
        }),
    });
}

fn required_gas(fees: Amount, gas_price: Amount, token_exchange_rate: u128) -> Result<Amount> {
    type U256 = ruint::Uint<256, 4>;

    let fees = U256::try_from(fees.0).unwrap();
    let gas_price = U256::try_from(gas_price.0).unwrap();
    let token_exchange_rate = U256::try_from(token_exchange_rate).unwrap();
    let token_exchange_rate_scale = U256::try_from(TOKEN_EXCHANGE_RATE_SCALE).unwrap();

    let dest_gas_cost = fees * gas_price;
    let gas_required = dest_gas_cost
        .checked_mul(token_exchange_rate)
        .ok_or(anyhow!("gas required mul overflow"))?
        .checked_div(token_exchange_rate_scale)
        .ok_or(anyhow!("token exchange scale rate is 0"))?;

    Ok(Amount(gas_required.try_into().map_err(|_| {
        anyhow::anyhow!("Amount may not exceed 2^128 - 1 after scaling")
    })?))
}

#[test]
fn send_tokens_with_metadata_and_claim_on_proper_and_unauthorized_beneficiary() {
    let (mut runner, admin, user, relayer, beneficiary_account, another_user_account) = setup();

    let mut initial_beneficiary_bank_balance = Amount::ZERO;
    let bank = Bank::<S>::default();

    runner.query_visible_state(|state| {
        let beneficiary_bank_balance = bank
            .get_balance_of(&beneficiary_account.address(), config_gas_token_id(), state)
            .expect("beneficiary funds get")
            .unwrap_or_default();
        initial_beneficiary_bank_balance = beneficiary_bank_balance;
    });

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    let relayer_address = relayer.address();

    // Create oracle data
    let oracle_data = ExchangeRateAndGasPrice {
        gas_price: Amount(2),
        token_exchange_rate: TOKEN_EXCHANGE_RATE_SCALE, // 1.0
    };
    let domain_oracle_data = HashMap::from([(domain, oracle_data)]);

    // Set up domain default gas
    let default_gas = Amount(2000);
    let domain_default_gas = HashMap::from([(domain, default_gas)]);

    // Set relayer config using the helper function
    set_relayer_config(
        &mut runner,
        &relayer,
        domain_oracle_data.clone(),
        domain_default_gas,
        default_gas,
        Some(beneficiary_account.address()),
    );

    // bigger than gas_price * default_gas
    let gas_payment_limit = Amount(5000);

    // Calculate expected gas using the helper function
    let expected_required_gas = required_gas(
        default_gas,
        oracle_data.gas_price,
        oracle_data.token_exchange_rate,
    )
    .unwrap();

    let message_body = b"Hello, world!";
    let recipient_address: HexHash = [5u8; 32].into();
    let expected_message = Message {
        version: MESSAGE_VERSION,
        nonce: 0,
        origin_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        sender: user.address().to_sender(),
        dest_domain: domain,
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };

    let expected_message_id = expected_message.id(&mut unlimited_gas_meter()).unwrap();

    // Send message for dispatch
    {
        register_recipient(&mut runner, &admin, recipient_address);

        // Create IGP metadata using the helper struct
        let igp_metadata = IGPMetadata {
            destination_gas_limit: default_gas,
        };

        let bank = Bank::<S>::default();
        let igp = InterchainGasPaymaster::<S>::default();
        runner.execute_transaction(TransactionTestCase {
            input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
                domain,
                recipient: recipient_address,
                body: HexString(message_body.to_vec().try_into().unwrap()),
                metadata: Some(HexString(igp_metadata.serialize())),
                relayer: Some(relayer_address),
                gas_payment_limit,
            }),
            assert: Box::new(move |result, state| {
                assert!(result.events.iter().any(|event| {
                    matches!(
                        event,
                        TestRuntimeEvent::Mailbox(MailboxEvent::DispatchId { .. })
                    )
                }));

                assert!(result.events.iter().any(|event| {
                    matches!(
                        event,
                        TestRuntimeEvent::InterchainGasPaymaster(
                            InterchainGasPaymasterEvent::GasPayment {
                                relayer,
                                message_id,
                                dest_domain,
                                gas_limit,
                                payment
                            }
                        )
                        if relayer == &relayer_address
                        && message_id == &expected_message_id
                        && dest_domain == &domain
                        && gas_limit == &default_gas
                        && payment == &expected_required_gas
                    )
                }));

                let module_bank_balance = bank
                    .get_balance_of(igp.id.to_payable(), config_gas_token_id(), state)
                    .expect("igp module funds retrieved");
                assert_eq!(module_bank_balance, Some(expected_required_gas));

                let relayer_funds = igp
                    .funds
                    .get(&relayer_address, state)
                    .expect("relayer funds retrieved");
                assert_eq!(relayer_funds, Some(expected_required_gas));
            }),
        });
    }

    // Unauthorized beneficiary tries to claim rewards
    {
        runner.execute_transaction(TransactionTestCase {
            input: another_user_account.create_plain_message::<RT, InterchainGasPaymaster<S>>(
                InterchainGasPaymasterCallMessage::ClaimRewards { relayer_address },
            ),
            assert: Box::new(move |result, _| match result.tx_receipt {
                TxEffect::Reverted(_) => {}
                _ => {
                    panic!("Unexpected tx receipt: {:?}", result.tx_receipt);
                }
            }),
        });
    }

    // Authorized beneficiary claims rewards
    {
        let beneficiary_address = beneficiary_account.address();

        let bank = Bank::<S>::default();
        let igp = InterchainGasPaymaster::<S>::default();
        runner.execute_transaction(TransactionTestCase {
            input: beneficiary_account.create_plain_message::<RT, InterchainGasPaymaster<S>>(
                InterchainGasPaymasterCallMessage::ClaimRewards { relayer_address },
            ),
            assert: Box::new(move |result, state| {
                assert!(result.events.iter().any(|event| {
                    matches!(
                        event,
                        TestRuntimeEvent::InterchainGasPaymaster(
                            InterchainGasPaymasterEvent::RewardsClaimed {
                                beneficiary,
                                relayer
                            }
                        )
                    if beneficiary == &beneficiary_address && relayer == &relayer_address
                    )
                }));

                let module_bank_balance = bank
                    .get_balance_of(igp.id.to_payable(), config_gas_token_id(), state)
                    .expect("igp module funds retrieved");
                assert_eq!(module_bank_balance, Some(Amount::ZERO));

                let relayer_funds = igp
                    .funds
                    .get(&relayer_address, state)
                    .expect("relayer funds retrieved");

                assert_eq!(relayer_funds, Some(Amount::ZERO));

                let beneficiary_bank_balance = bank
                    .get_balance_of(&beneficiary_account.address(), config_gas_token_id(), state)
                    .expect("beneficiary funds retrieved")
                    .unwrap_or_default();

                assert_eq!(
                    beneficiary_bank_balance,
                    initial_beneficiary_bank_balance
                        .checked_add(expected_required_gas)
                        .unwrap()
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                );
            }),
        });
    }
}

#[test]
fn send_tokens_without_gas_limit_and_domain_has_default_gas() {
    let (mut runner, admin, user, relayer, beneficiary_account, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    let relayer_address = relayer.address();

    // Create oracle data
    let oracle_data = ExchangeRateAndGasPrice {
        gas_price: Amount(2),
        token_exchange_rate: TOKEN_EXCHANGE_RATE_SCALE, // 1.0
    };
    let domain_oracle_data = HashMap::from([(domain, oracle_data)]);

    // Set up domain default gas
    let domain_default_gas_value = Amount(6000);
    let domain_default_gas = HashMap::from([(domain, domain_default_gas_value)]);
    let default_gas = Amount(2000);

    // Set relayer config using the helper function
    set_relayer_config(
        &mut runner,
        &relayer,
        domain_oracle_data.clone(),
        domain_default_gas,
        default_gas,
        Some(beneficiary_account.address()),
    );

    // Calculate expected gas using the helper function
    let expected_required_gas = required_gas(
        domain_default_gas_value,
        oracle_data.gas_price,
        oracle_data.token_exchange_rate,
    )
    .unwrap();

    // Send message for dispatch
    {
        let recipient_address: HexHash = [5u8; 32].into();
        register_recipient(&mut runner, &admin, recipient_address);

        let message_body = b"Hello, world!";

        let igp = InterchainGasPaymaster::<S>::default();
        runner.execute_transaction(TransactionTestCase {
            input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
                domain,
                recipient: recipient_address,
                body: HexString(message_body.to_vec().try_into().unwrap()),
                metadata: Some(HexString(SafeVec::default())),
                relayer: Some(relayer_address),
                gas_payment_limit: Amount::MAX,
            }),
            assert: Box::new(move |result, state| {
                assert!(result.events.iter().any(|event| {
                    matches!(
                        event,
                        TestRuntimeEvent::Mailbox(MailboxEvent::DispatchId { .. })
                    )
                }));

                let relayer_funds = igp
                    .funds
                    .get(&relayer_address, state)
                    .expect("relayer funds retrieved");
                assert_eq!(relayer_funds, Some(expected_required_gas));
            }),
        });
    }
}

#[test]
fn send_tokens_with_malformed_metadata_limit_and_relayer_has_only_default_gas_set() {
    let (mut runner, admin, user, relayer, beneficiary_account, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    let relayer_address = relayer.address();

    // Create oracle data
    let oracle_data = ExchangeRateAndGasPrice {
        gas_price: Amount(2),
        token_exchange_rate: TOKEN_EXCHANGE_RATE_SCALE, // 1.0
    };
    let domain_oracle_data = HashMap::from([(domain, oracle_data)]);

    // Set up domain default gas
    let domain_default_gas = HashMap::new();
    let default_gas = Amount(2000);

    // Set relayer config using the helper function
    set_relayer_config(
        &mut runner,
        &relayer,
        domain_oracle_data.clone(),
        domain_default_gas,
        default_gas,
        Some(beneficiary_account.address()),
    );

    // Calculate expected gas using the helper function
    let expected_required_gas = required_gas(
        default_gas,
        oracle_data.gas_price,
        oracle_data.token_exchange_rate,
    )
    .unwrap();

    // Send message for dispatch
    {
        let recipient_address: HexHash = [5u8; 32].into();
        register_recipient(&mut runner, &admin, recipient_address);

        let message_body = b"Hello, world!";

        let mut metadata = SafeVec::default();
        metadata.try_push(3u8).unwrap();

        let igp = InterchainGasPaymaster::<S>::default();
        runner.execute_transaction(TransactionTestCase {
            input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
                domain,
                recipient: recipient_address,
                body: HexString(message_body.to_vec().try_into().unwrap()),
                metadata: Some(HexString(metadata)),
                relayer: Some(relayer_address),
                gas_payment_limit: Amount::MAX,
            }),
            assert: Box::new(move |result, state| {
                assert!(result.events.iter().any(|event| {
                    matches!(
                        event,
                        TestRuntimeEvent::Mailbox(MailboxEvent::DispatchId { .. })
                    )
                }));

                let relayer_funds = igp
                    .funds
                    .get(&relayer_address, state)
                    .expect("relayer funds retrieved");

                assert_eq!(relayer_funds, Some(expected_required_gas));
            }),
        });
    }
}

#[test]
fn send_tokens_with_u256_limit_and_relayer_has_only_default_gas_set() {
    let (mut runner, admin, user, relayer, beneficiary_account, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    let relayer_address = relayer.address();

    // Create oracle data
    let oracle_data = ExchangeRateAndGasPrice {
        gas_price: Amount(2),
        token_exchange_rate: TOKEN_EXCHANGE_RATE_SCALE, // 1.0
    };
    let domain_oracle_data = HashMap::from([(domain, oracle_data)]);

    // Set up domain default gas
    let domain_default_gas = HashMap::new();
    let default_gas = Amount(2000);

    // Set relayer config using the helper function
    set_relayer_config(
        &mut runner,
        &relayer,
        domain_oracle_data.clone(),
        domain_default_gas,
        default_gas,
        Some(beneficiary_account.address()),
    );

    // Calculate expected gas using the helper function
    let expected_required_gas = required_gas(
        default_gas,
        oracle_data.gas_price,
        oracle_data.token_exchange_rate,
    )
    .unwrap();

    // Send message for dispatch
    {
        let recipient_address: HexHash = [5u8; 32].into();
        register_recipient(&mut runner, &admin, recipient_address);

        let message_body = b"Hello, world!";

        // Simulate u256 set as gas limit, should fallback to default gas
        let mut buf = vec![0_u8; 86];
        // Set the first byte of the gas limit to non-zero
        buf[34] = 1;
        // Fill the rest with valid data
        buf[34 + 16..34 + 32].copy_from_slice(&u128::MAX.to_be_bytes());

        let metadata = SafeVec::try_from(buf).expect("init metadata");

        let igp = InterchainGasPaymaster::<S>::default();
        runner.execute_transaction(TransactionTestCase {
            input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
                domain,
                recipient: recipient_address,
                body: HexString(message_body.to_vec().try_into().unwrap()),
                metadata: Some(HexString(metadata)),
                relayer: Some(relayer_address),
                gas_payment_limit: Amount::MAX,
            }),
            assert: Box::new(move |result, state| {
                assert!(result.events.iter().any(|event| {
                    matches!(
                        event,
                        TestRuntimeEvent::Mailbox(MailboxEvent::DispatchId { .. })
                    )
                }));

                let relayer_funds = igp
                    .funds
                    .get(&relayer_address, state)
                    .expect("relayer funds retrieved");

                assert_eq!(relayer_funds, Some(expected_required_gas));
            }),
        });
    }
}

#[test]
fn send_tokens_with_insufficient_gas() {
    let (mut runner, admin, user, relayer, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    let relayer_address = relayer.address();

    // Create oracle data
    let domain_oracle_data = HashMap::from([(
        domain,
        ExchangeRateAndGasPrice {
            gas_price: Amount(2),
            token_exchange_rate: TOKEN_EXCHANGE_RATE_SCALE, // 1.0
        },
    )]);

    // Set up domain default gas
    let default_gas = Amount(2000);
    let domain_default_gas = HashMap::from([(domain, default_gas)]);

    // Set relayer config using the helper function
    set_relayer_config(
        &mut runner,
        &relayer,
        domain_oracle_data.clone(),
        domain_default_gas,
        default_gas,
        None,
    );

    // required payment is default_gas (2000) * gas_price (2) * gas_scale (1.0)
    let gas_payment_limit = Amount(3999);

    // Send message for dispatch
    let recipient_address: HexHash = [5u8; 32].into();
    register_recipient(&mut runner, &admin, recipient_address);

    let igp_metadata = IGPMetadata {
        destination_gas_limit: default_gas,
    };
    let message_body = b"Hello, world!";

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
            domain,
            recipient: recipient_address,
            body: HexString(message_body.to_vec().try_into().unwrap()),
            metadata: Some(HexString(igp_metadata.serialize())),
            relayer: Some(relayer_address),
            gas_payment_limit,
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(..) => {}
            _ => {
                panic!("Unexpected tx receipt: {:?}", result.tx_receipt);
            }
        }),
    });

    // Now check if it succeeds with the expected required gas
    let gas_payment_limit = Amount(4000);

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
            domain,
            recipient: recipient_address,
            body: HexString(message_body.to_vec().try_into().unwrap()),
            metadata: Some(HexString(igp_metadata.serialize())),
            relayer: Some(relayer_address),
            gas_payment_limit,
        }),
        assert: Box::new(move |result, _| {
            if let TxEffect::Reverted(..) = result.tx_receipt {
                panic!("Tx reverted even tho gas payment should suffice")
            }
        }),
    });
}

#[test]
fn send_tokens_with_0_limit_and_relayer_has_only_default_gas_set() {
    let (mut runner, admin, user, relayer, beneficiary_account, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    let relayer_address = relayer.address();

    // Create oracle data
    let oracle_data = ExchangeRateAndGasPrice {
        gas_price: Amount(2),
        token_exchange_rate: TOKEN_EXCHANGE_RATE_SCALE, // 1.0
    };
    let domain_oracle_data = HashMap::from([(domain, oracle_data)]);

    // Set up domain default gas
    let domain_default_gas = HashMap::new();
    let default_gas = Amount(2000);

    // Set relayer config using the helper function
    set_relayer_config(
        &mut runner,
        &relayer,
        domain_oracle_data.clone(),
        domain_default_gas,
        default_gas,
        Some(beneficiary_account.address()),
    );

    // Calculate expected gas using the helper function
    let expected_gas_required = required_gas(
        default_gas,
        oracle_data.gas_price,
        oracle_data.token_exchange_rate,
    )
    .unwrap();

    // Send message for dispatch
    {
        let recipient_address: HexHash = [5u8; 32].into();
        register_recipient(&mut runner, &admin, recipient_address);

        let message_body = b"Hello, world!";

        let mut metadata = SafeVec::default();
        metadata.try_push(3u8).unwrap();

        let igp = InterchainGasPaymaster::<S>::default();
        runner.execute_transaction(TransactionTestCase {
            input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
                domain,
                recipient: recipient_address,
                body: HexString(message_body.to_vec().try_into().unwrap()),
                metadata: Some(HexString(metadata)),
                relayer: Some(relayer_address),
                gas_payment_limit: Amount::MAX,
            }),
            assert: Box::new(move |result, state| {
                assert!(result.events.iter().any(|event| {
                    matches!(
                        event,
                        TestRuntimeEvent::Mailbox(MailboxEvent::DispatchId { .. })
                    )
                }));

                let relayer_funds = igp
                    .funds
                    .get(&relayer_address, state)
                    .expect("relayer funds retrieved");

                assert_eq!(relayer_funds, Some(expected_gas_required));
            }),
        });
    }
}

#[test]
fn transfer_with_various_gas_prices_and_scales() {
    #[derive(Debug)]
    struct TestCase {
        gas_price: Amount,
        exchange_rate: u128,
        destination_gas: Amount,
        error: String,
    }

    // error that signals that admin didn't have enough funds to pay the gas price
    // it is a "success" error for tests that check possible overflows during gas calculation
    let not_enough_funds_err = "Failed to transfer token";

    // gas_price * desination_gas overflow u128, brought back to u128 by exchange_rate
    let big_destination_gas_price_overflows_u128 = TestCase {
        gas_price: Amount::MAX.checked_div(Amount(2)).unwrap(),
        exchange_rate: TOKEN_EXCHANGE_RATE_SCALE / 10, // 0.1
        destination_gas: Amount(3),
        error: not_enough_funds_err.to_string(),
    };

    // gas_price * desination_gas overflow u128, brought back to u128 by exchange_rate
    let big_destination_gas_needed_overflows_u128 = TestCase {
        gas_price: Amount(3),
        exchange_rate: TOKEN_EXCHANGE_RATE_SCALE / 10, // 0.1
        destination_gas: Amount::MAX.checked_div(Amount(2)).unwrap(),
        error: not_enough_funds_err.to_string(),
    };

    // overflow u128 calculating the gas needed
    let final_gas_needed_overflows_u128 = TestCase {
        gas_price: Amount(TOKEN_EXCHANGE_RATE_SCALE), // this counters final division by rate scale
        exchange_rate: u128::MAX,                     // ~ 10e19
        destination_gas: Amount::MAX.checked_div(Amount(5)).unwrap(),
        error: "failed preparing quote".to_string(),
    };

    for tc in [
        big_destination_gas_price_overflows_u128,
        big_destination_gas_needed_overflows_u128,
        final_gas_needed_overflows_u128,
    ] {
        let (mut runner, admin, user, relayer, beneficiary_account, ..) = setup();

        let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
        let relayer_address = relayer.address();

        // Set relayer config
        let oracle_data = ExchangeRateAndGasPrice {
            gas_price: tc.gas_price,
            token_exchange_rate: tc.exchange_rate,
        };
        let domain_oracle_data = HashMap::from([(domain, oracle_data)]);
        let domain_default_gas = HashMap::from([(domain, tc.destination_gas)]);
        let default_gas = tc.destination_gas;
        set_relayer_config(
            &mut runner,
            &relayer,
            domain_oracle_data.clone(),
            domain_default_gas,
            default_gas,
            Some(beneficiary_account.address()),
        );

        // Send message for dispatch
        let recipient_address: HexHash = [5u8; 32].into();
        register_recipient(&mut runner, &admin, recipient_address);
        runner.execute_transaction(TransactionTestCase {
            input: user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
                domain,
                recipient: recipient_address,
                body: HexString(b"Hello".to_vec().try_into().unwrap()),
                metadata: None,
                relayer: Some(relayer_address),
                gas_payment_limit: Amount::MAX,
            }),
            assert: Box::new(move |result, _| {
                if let TxEffect::Reverted(reverted) = result.tx_receipt {
                    assert!(
                        reverted.reason.to_string().contains(&tc.error),
                        "Unexpected revert reason: {}; case: {tc:?}",
                        reverted.reason
                    );
                } else {
                    panic!("Expected tx to be reverted but it succeeded; case: {tc:?}");
                }
            }),
        });
    }
}

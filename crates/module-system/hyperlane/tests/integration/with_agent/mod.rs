//! End to end tests for hyperlane implementation that utilize real relayer, validators and evm devnet.
//!
//! The docker setup uses a single container which provides all the needed tools.
//! It comes from <https://github.com/eigerco/hyperlane-monorepo/blob/main/hyperlane.Dockerfile>.
//!
//! To build it:
//! ```bash
//! git clone https://github.com/eigerco/hyperlane-monorepo
//! cd hyperlane-monorepo
//! ./build.sh
//! ```
//!
//! Tests will always fetch the latest shipped image. To run tests with locally built image:
//! ```bash
//! export CUSTOM_HLP_DOCKER_IMAGE=hyperlane
//! ```
//!
//! For more information about the setup, check the [`HyperlaneBuilder`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use futures::Stream;
use helpers::{
    generate_setup, parse_eth_addr, setup_rollup, HyperlaneBuilder, ANVIL_ACCOUNTS,
    DEFAULT_FINALIZATION_BLOCKS, EVM_DOMAIN,
};
use preferred_sequencer_runtime::{TestRuntime, TestRuntimeCall};
use serde_json::{Map, Value};
use sov_api_spec::types::{self as api_types, IntOrHash, LedgerEvent, Slot};
use sov_api_spec::Client;
use sov_bank::Amount;
use sov_hyperlane_integration::igp::ExchangeRateAndGasPrice;
use sov_hyperlane_integration::warp::{self, Admin, TokenKind};
use sov_hyperlane_integration::{
    test_recipient, CallMessage, HyperlaneAddress, InterchainGasPaymasterCallMessage, Ism,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::{
    CryptoSpec, DispatchCall, HexHash, HexString, RawTx, Runtime, SafeVec, Spec,
};
use sov_test_utils::{default_test_signed_transaction, TestSpec, TestUser};
use tokio::time::sleep;
use tokio_stream::StreamExt;

use crate::igp::{default_gas_hashmap_to_safe_vec, oracle_data_hashmap_to_safe_vec};

mod configs;
mod helpers;
mod preferred_sequencer_runtime;

#[tokio::test(flavor = "multi_thread")]
async fn test_validator_announces_itself() {
    sov_test_utils::initialize_logging();
    let dir = tempfile::tempdir().unwrap();
    let builder = HyperlaneBuilder::setup_image().await;
    let setup = generate_setup();
    let validator = setup.validators[0].clone();
    let rollup = setup_rollup(dir.path().to_path_buf(), setup, false).await;

    let mut slot_subscription = rollup.api_client.subscribe_slots().await.unwrap();

    let mut hyperlane = builder
        .with_rollup_port(rollup.http_addr.port())
        .with_validators([&validator])
        .start()
        .await;

    // wait for the first finalized block
    for i in 0..DEFAULT_FINALIZATION_BLOCKS * 30 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;
        println!("ROUND {i}, events: {events:?}");
        if let Some(process_event) = find_event(&events, "Mailbox/ValidatorAnnouncement") {
            assert_eq!(
                process_event["validator_announcement"]["address"],
                ANVIL_ACCOUNTS[1].0.to_string(),
            );
            assert!(process_event["validator_announcement"]["storage_location"]
                .as_str()
                .unwrap()
                .starts_with("file:///validator0/signatures"));

            rollup.shutdown().await.unwrap();
            return;
        }
    }

    rollup.shutdown().await.unwrap();
    hyperlane.print_stdout().await;
    panic!("Mailbox/ValidatorAnnouncement event not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_relayer_basic_dispatch_process() {
    let dir = tempfile::tempdir().unwrap();
    let builder = HyperlaneBuilder::setup_image().await;
    let setup = generate_setup();
    let relayer = setup.relayer.clone();
    let prover = setup.prover.clone();
    let prover_addr = prover.user_info.address();
    let rollup = setup_rollup(dir.path().to_path_buf(), setup, true).await;

    let mut hyperlane = builder
        .with_rollup_port(rollup.http_addr.port())
        .with_relayer(&relayer)
        .start()
        .await;

    // wait for first finalized block
    let mut slot_subscription = rollup.api_client.subscribe_slots().await.unwrap();
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS {
        slot_subscription.next().await.unwrap().unwrap();
    }

    // set relayer igp config
    let relayer_config_tx = tx_set_relayer_config(&relayer);
    submit_tx(&rollup.api_client, relayer_config_tx).await;

    // register prover as a recipient
    let register_call = TestRuntimeCall::TestRecipient(test_recipient::CallMessage::Register {
        address: prover_addr.to_sender(),
        ism: Ism::AlwaysTrust,
    });
    let register_tx = encode_call(prover.user_info.private_key(), &register_call);
    submit_tx(&rollup.api_client, register_tx).await;

    // dispatch message to prover
    let dispatch_tx = tx_send_message(&relayer, prover_addr.to_sender(), None, b"Hello there");
    submit_tx(&rollup.api_client, dispatch_tx).await;

    // look for `process` event
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS * 15 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;

        if let Some(process_event) = find_event(&events, "Mailbox/Process") {
            assert_eq!(
                process_event["process"]["recipient_address"],
                prover_addr.to_sender().to_string(),
            );
            assert_eq!(
                process_event["process"]["sender_address"],
                relayer.address().to_sender().to_string(),
            );

            let test_recipient_event =
                find_event(&events, "TestRecipient/MessageReceivedGeneric").unwrap();
            assert_eq!(
                test_recipient_event["MessageReceivedGeneric"]["body"],
                format!("0x{}", hex::encode("Hello there"))
            );

            rollup.shutdown().await.unwrap();
            return;
        }
    }

    rollup.shutdown().await.unwrap();
    hyperlane.print_stdout().await;
    panic!("Mailbox/Process event not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multisig_ism() {
    let dir = tempfile::tempdir().unwrap();
    let builder = HyperlaneBuilder::setup_image().await;
    let setup = generate_setup();
    let relayer = setup.relayer.clone();
    let prover = setup.prover.clone();
    let prover_addr = prover.user_info.address();
    let validators = setup.validators.clone();
    let rollup = setup_rollup(dir.path().to_path_buf(), setup, true).await;

    let mut hyperlane = builder
        .with_rollup_port(rollup.http_addr.port())
        .with_relayer(&relayer)
        .with_validators(&validators[..2])
        .start()
        .await;

    // wait for first finalized block
    let mut slot_subscription = rollup.api_client.subscribe_slots().await.unwrap();
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS {
        slot_subscription.next().await.unwrap().unwrap();
    }

    // set relayer igp config
    let relayer_config_tx = tx_set_relayer_config(&relayer);
    submit_tx(&rollup.api_client, relayer_config_tx).await;

    // register prover as a recipient with first 3 validators addresses for multisig
    let val_addresses: Vec<_> = ANVIL_ACCOUNTS[1..4]
        .iter()
        .map(|(addr, _)| addr.parse().unwrap())
        .collect();
    let register_call = TestRuntimeCall::TestRecipient(test_recipient::CallMessage::Register {
        address: prover_addr.to_sender(),
        ism: Ism::MessageIdMultisig {
            validators: val_addresses.try_into().unwrap(),
            threshold: 2,
        },
    });
    let register_tx = encode_call(prover.user_info.private_key(), &register_call);
    submit_tx(&rollup.api_client, register_tx).await;

    // dispatch message to prover
    let dispatch_tx = tx_send_message(&relayer, prover_addr.to_sender(), None, b"Hello there");
    submit_tx(&rollup.api_client, dispatch_tx).await;

    // look for `process` event
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS * 15 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;

        if let Some(process_event) = find_event(&events, "Mailbox/Process") {
            assert_eq!(
                process_event["process"]["recipient_address"],
                prover_addr.to_sender().to_string(),
            );
            assert_eq!(
                process_event["process"]["sender_address"],
                relayer.address().to_sender().to_string(),
            );

            rollup.shutdown().await.unwrap();
            return;
        }
    }

    rollup.shutdown().await.unwrap();
    hyperlane.print_stdout().await;
    panic!("Mailbox/Process event not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_process_message_from_evm_counterparty() {
    let dir = tempfile::tempdir().unwrap();
    let builder = HyperlaneBuilder::setup_image().await;
    let setup = generate_setup();
    let relayer = setup.relayer.clone();
    let prover = setup.prover.clone();
    let prover_addr = prover.user_info.address();
    let rollup = setup_rollup(dir.path().to_path_buf(), setup, true).await;

    let mut hyperlane = builder
        .with_rollup_port(rollup.http_addr.port())
        .with_relayer(&relayer)
        .with_evm_counterparty(prover_addr.to_sender())
        .start()
        .await;

    // wait for first finalized block
    let mut slot_subscription = rollup.api_client.subscribe_slots().await.unwrap();
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS {
        slot_subscription.next().await.unwrap().unwrap();
    }

    // register prover as a recipient
    let register_call = TestRuntimeCall::TestRecipient(test_recipient::CallMessage::Register {
        address: prover_addr.to_sender(),
        ism: Ism::AlwaysTrust,
    });
    let register_tx = encode_call(prover.user_info.private_key(), &register_call);
    submit_tx(&rollup.api_client, register_tx).await;

    // dispatch test message to prover from evm
    let (expected_message, expected_message_id) = hyperlane.dispatch_msg_from_counterparty().await;
    let sender_addr = parse_eth_addr(ANVIL_ACCOUNTS[0].0);

    assert_eq!(expected_message.origin_domain, EVM_DOMAIN);
    assert_eq!(
        expected_message.dest_domain,
        config_value!("HYPERLANE_BRIDGE_DOMAIN")
    );
    assert_eq!(expected_message.sender, sender_addr);
    assert_eq!(expected_message.recipient, prover_addr.to_sender());

    // finalize the block with dispatched message
    hyperlane.mine_next_block_on_counterparty().await;

    // look for `process` event
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS * 15 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;

        if let Some(process_event) = find_event(&events, "Mailbox/Process") {
            assert_eq!(
                process_event["process"]["recipient_address"],
                prover_addr.to_sender().to_string(),
            );
            assert_eq!(
                process_event["process"]["sender_address"],
                sender_addr.to_string(),
            );

            let test_recipient_event =
                find_event(&events, "TestRecipient/MessageReceivedGeneric").unwrap();
            assert_eq!(
                test_recipient_event["MessageReceivedGeneric"]["body"],
                expected_message.body.to_string(),
            );

            let process_id_event = find_event(&events, "Mailbox/ProcessId").unwrap();
            assert_eq!(
                process_id_event["process_id"]["id"],
                expected_message_id.to_string()
            );

            rollup.shutdown().await.unwrap();
            return;
        }
    }

    rollup.shutdown().await.unwrap();
    hyperlane.print_stdout().await;
    panic!("Mailbox/Process event not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_dispatch_message_to_evm_counterparty() {
    let dir = tempfile::tempdir().unwrap();
    let builder = HyperlaneBuilder::setup_image().await;
    let setup = generate_setup();
    let relayer = setup.relayer.clone();
    let prover = setup.prover.clone();
    let prover_addr = prover.user_info.address();
    let rollup = setup_rollup(dir.path().to_path_buf(), setup, true).await;

    let mut hyperlane = builder
        .with_rollup_port(rollup.http_addr.port())
        .with_relayer(&relayer)
        .with_evm_counterparty(prover_addr.to_sender())
        .start()
        .await;

    // wait for first finalized block
    let mut slot_subscription = rollup.api_client.subscribe_slots().await.unwrap();
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS {
        slot_subscription.next().await.unwrap().unwrap();
    }

    // set relayer igp config
    let relayer_config_tx = tx_set_relayer_config(&relayer);
    submit_tx(&rollup.api_client, relayer_config_tx).await;

    let evm_recipient = hyperlane.evm_recipient.unwrap();
    // dispatch message to evm test recipient
    let dispatch_tx = tx_send_message(&relayer, evm_recipient, Some(EVM_DOMAIN), b"Hello there");
    submit_tx(&rollup.api_client, dispatch_tx).await;

    // look for `dispatch` event
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS * 15 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;

        if let Some(process_event) = find_event(&events, "Mailbox/Dispatch") {
            assert_eq!(process_event["dispatch"]["destination_domain"], EVM_DOMAIN,);
            assert_eq!(
                process_event["dispatch"]["recipient_address"],
                evm_recipient.to_string(),
            );

            let message_id_event = find_event(&events, "Mailbox/DispatchId").unwrap();
            let message_id = message_id_event["dispatch_id"]["id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();

            // Find the dispatched message on counterparty
            sleep(Duration::from_secs(10)).await; // give relayer extra time to relay
            let evm_event = hyperlane.latest_message_on_counterparty().await;
            assert_eq!(
                evm_event.origin_domain,
                config_value!("HYPERLANE_BRIDGE_DOMAIN")
            );
            assert_eq!(evm_event.sender_address, relayer.address().to_sender());
            assert_eq!(evm_event.recipient_address, evm_recipient);
            assert_eq!(evm_event.id, message_id);

            rollup.shutdown().await.unwrap();
            return;
        }
    }

    rollup.shutdown().await.unwrap();
    hyperlane.print_stdout().await;
    panic!("Mailbox/Dispatch event not found");
}

async fn test_warp_transfer_back_and_forth_with_evm_counterparty(
    local_decimals: u8,
    inbound_sent_amount: Amount,
    inbound_received_amount: Amount,
    outbound_sent_amount: Amount,
    outbound_received_amount: Amount,
) {
    let dir = tempfile::tempdir().unwrap();
    let builder = HyperlaneBuilder::setup_image().await;
    let setup = generate_setup();
    let relayer = setup.relayer.clone();
    let prover = setup.prover.clone();
    let prover_addr = prover.user_info.address();
    let rollup = setup_rollup(dir.path().to_path_buf(), setup, true).await;

    let mut hyperlane = builder
        .with_rollup_port(rollup.http_addr.port())
        .with_relayer(&relayer)
        .with_evm_counterparty(prover_addr.to_sender())
        .start()
        .await;

    // wait for first finalized block
    let mut slot_subscription = rollup.api_client.subscribe_slots().await.unwrap();
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS {
        slot_subscription.next().await.unwrap().unwrap();
    }

    // set relayer igp config
    let relayer_config_tx = tx_set_relayer_config(&relayer);
    submit_tx(&rollup.api_client, relayer_config_tx).await;

    // native ETH doesn't have its own address, so let's create a dummy one
    let addr = format!(
        "{}_{}_warp_nativeETH",
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        EVM_DOMAIN
    );
    let mut token_addr_on_counterparty = [0; 32];
    token_addr_on_counterparty[32 - addr.len()..].copy_from_slice(addr.as_bytes());
    let token_addr_on_counterparty = HexString(token_addr_on_counterparty);

    // register warp route for nativeETH
    let register_call = TestRuntimeCall::Warp(warp::CallMessage::Register {
        admin: Admin::InsecureOwner(relayer.address()),
        token_source: TokenKind::Synthetic {
            // use the warp route address for token id
            remote_token_id: token_addr_on_counterparty,
            local_decimals: Some(local_decimals),
            remote_decimals: 18,
        },
        ism: Ism::AlwaysTrust,
        remote_routers: SafeVec::new(),
        inbound_transferrable_tokens_limit: Amount::MAX,
        inbound_limit_replenishment_per_slot: Amount::MAX,
        outbound_transferrable_tokens_limit: Amount::MAX,
        outbound_limit_replenishment_per_slot: Amount::MAX,
    });
    let register_tx = encode_call(relayer.private_key(), &register_call);
    submit_tx(&rollup.api_client, register_tx).await;

    let mut route_deployed = false;
    let mut local_route_id = HexString([0; 32]);
    let mut remote_route_id = HexString([0; 32]);
    // look for `route registered` event
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS * 15 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;

        if let Some(route_registered_event) = find_event(&events, "Warp/RouteRegistered") {
            assert_eq!(
                route_registered_event["route_registered"]["token_source"]["Synthetic"]
                    ["remote_token_id"],
                token_addr_on_counterparty.to_string()
            );
            local_route_id = route_registered_event["route_registered"]["route_id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();

            // deploy warp route on counterparty
            remote_route_id = hyperlane
                .deploy_warp_route_on_counterparty(local_route_id, local_decimals)
                .await;

            // enroll remote router on rollup
            let enroll_router_call = TestRuntimeCall::Warp(warp::CallMessage::EnrollRemoteRouter {
                warp_route: local_route_id,
                remote_domain: EVM_DOMAIN,
                remote_router_address: remote_route_id,
            });
            let enroll_router_tx = encode_call(relayer.private_key(), &enroll_router_call);
            submit_tx(&rollup.api_client, enroll_router_tx).await;

            route_deployed = true;
            break;
        }
    }

    if !route_deployed {
        rollup.shutdown().await.unwrap();
        hyperlane.print_stdout().await;
        panic!("Warp/RouteRegistered event not found");
    }

    // transfer native eth from evm counterparty to prover
    let prover_addr = prover.user_info.address().to_sender();
    hyperlane
        .send_warp_token_transfer_from_counterparty(prover_addr, inbound_sent_amount)
        .await;
    hyperlane.mine_next_block_on_counterparty().await;

    let mut transfer_received = false;
    for _ in 0..DEFAULT_FINALIZATION_BLOCKS * 15 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;

        if let Some(token_recv_event) = find_event(&events, "Warp/TokenTransferReceived") {
            assert_eq!(
                token_recv_event["token_transfer_received"]["route_id"],
                local_route_id.to_string()
            );
            assert_eq!(
                token_recv_event["token_transfer_received"]["from_domain"],
                EVM_DOMAIN
            );
            assert_eq!(
                token_recv_event["token_transfer_received"]["recipient"],
                prover_addr.to_string()
            );
            assert_eq!(
                token_recv_event["token_transfer_received"]["amount"],
                inbound_received_amount.to_string(),
            );

            transfer_received = true;
            break;
        }
    }

    if !transfer_received {
        rollup.shutdown().await.unwrap();
        hyperlane.print_stdout().await;
        panic!("Warp/TokenTransferReceived event not found");
    }

    // Now transfer it back to the remote
    // choose some account which didn't have to pay for anything yet
    // to simplify balance asserts
    let evm_recipient = parse_eth_addr(ANVIL_ACCOUNTS[8].0);
    let recipient_balance_before_transfer = hyperlane.counterparty_balance_of(evm_recipient).await;
    assert_eq!(
        recipient_balance_before_transfer,
        10_000_000_000_000_000_000_000
    );

    let transfer_call = TestRuntimeCall::Warp(warp::CallMessage::TransferRemote {
        warp_route: local_route_id,
        destination_domain: EVM_DOMAIN,
        recipient: evm_recipient,
        amount: outbound_sent_amount,
        relayer: Some(relayer.address()),
        gas_payment_limit: Amount::MAX,
    });
    let transfer_tx = encode_call(prover.user_info.private_key(), &transfer_call);
    submit_tx(&rollup.api_client, transfer_tx).await;

    for _ in 0..DEFAULT_FINALIZATION_BLOCKS * 15 {
        let events = next_slot_events(&rollup.api_client, &mut slot_subscription).await;

        // look for event that sent outbound transfer
        if let Some(token_sent_event) = find_event(&events, "Warp/TokenTransferredRemote") {
            assert_eq!(
                token_sent_event["token_transferred_remote"]["route_id"],
                local_route_id.to_string()
            );
            assert_eq!(
                token_sent_event["token_transferred_remote"]["to_domain"],
                EVM_DOMAIN
            );
            assert_eq!(
                token_sent_event["token_transferred_remote"]["recipient"],
                evm_recipient.to_string()
            );
            let mut amount_hex = [0; 32];
            amount_hex[16..].copy_from_slice(&outbound_received_amount.0.to_be_bytes());
            assert_eq!(
                token_sent_event["token_transferred_remote"]["amount"],
                HexString(amount_hex).to_string()
            );

            // check if transfer was received by counterparty
            sleep(Duration::from_secs(10)).await; // give relayer extra time to relay
            let (origin_domain, recipient) = hyperlane
                .latest_warp_transfer_on_counterparty(remote_route_id)
                .await;
            assert_eq!(origin_domain, config_value!("HYPERLANE_BRIDGE_DOMAIN"));
            assert_eq!(recipient, evm_recipient);

            let recipient_balance_after_transfer =
                hyperlane.counterparty_balance_of(evm_recipient).await;
            assert_eq!(
                recipient_balance_before_transfer
                    .checked_add(outbound_received_amount)
                    .unwrap(),
                recipient_balance_after_transfer
            );

            rollup.shutdown().await.unwrap();
            return;
        }
    }

    rollup.shutdown().await.unwrap();
    hyperlane.print_stdout().await;
    panic!("Warp/TokenTransferredRemote event not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_warp_transfer_back_and_forth_with_evm_without_scaling() {
    test_warp_transfer_back_and_forth_with_evm_counterparty(
        18,
        Amount(1234),
        Amount(1234),
        Amount(1234),
        Amount(1234),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_warp_transfer_back_and_forth_with_evm_scaled_down() {
    test_warp_transfer_back_and_forth_with_evm_counterparty(
        16,
        Amount(1234),
        Amount(12),
        Amount(12),
        Amount(1200),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_warp_transfer_back_and_forth_with_evm_scaled_up() {
    test_warp_transfer_back_and_forth_with_evm_counterparty(
        20,
        Amount(1234),
        Amount(123400),
        Amount(123333),
        Amount(1233),
    )
    .await;
}

fn tx_send_message(
    relayer: &TestUser<TestSpec>,
    recipient_address: HexHash,
    domain: Option<u32>,
    message_body: &[u8],
) -> RawTx {
    let call = TestRuntimeCall::Mailbox(CallMessage::Dispatch {
        domain: domain.unwrap_or(config_value!("HYPERLANE_BRIDGE_DOMAIN")),
        recipient: recipient_address,
        body: HexString(message_body.to_vec().try_into().unwrap()),
        metadata: None,
        relayer: Some(relayer.address()),
        gas_payment_limit: Amount::MAX,
    });

    encode_call(relayer.private_key(), &call)
}

fn encode_call(
    key: &<<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    call_message: &<TestRuntime<TestSpec> as DispatchCall>::Decodable,
) -> RawTx {
    let tx = default_test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        call_message,
        nonce(),
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

async fn submit_tx(client: &Client, tx_body: RawTx) {
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_body),
        })
        .await
        .unwrap();
}

async fn next_slot_events<S>(client: &Client, subscription: &mut S) -> Vec<LedgerEvent>
where
    S: Stream<Item = Result<Slot>> + Unpin,
{
    let slot = subscription.next().await.unwrap().unwrap();
    client
        .get_slot_filtered_events(&IntOrHash::Integer(slot.number), None)
        .await
        .unwrap()
        .clone()
}

fn find_event(events: &[LedgerEvent], event: &str) -> Option<Map<String, Value>> {
    events
        .iter()
        .find(|ev| ev.key == event)
        .map(|ev| ev.value.clone())
}

fn tx_set_relayer_config(relayer: &TestUser<TestSpec>) -> RawTx {
    let default_gas = Amount(2000);
    let domain_oracles = HashMap::from([
        (
            config_value!("HYPERLANE_BRIDGE_DOMAIN"),
            ExchangeRateAndGasPrice {
                gas_price: Amount(1),
                token_exchange_rate: 1,
            },
        ),
        (
            EVM_DOMAIN,
            ExchangeRateAndGasPrice {
                gas_price: Amount(1),
                token_exchange_rate: 1,
            },
        ),
    ]);
    let domain_gas = HashMap::from([
        (config_value!("HYPERLANE_BRIDGE_DOMAIN"), default_gas),
        (EVM_DOMAIN, default_gas),
    ]);
    let call = TestRuntimeCall::InterchainGasPaymaster(
        InterchainGasPaymasterCallMessage::SetRelayerConfig {
            domain_oracle_data: oracle_data_hashmap_to_safe_vec(domain_oracles),
            domain_default_gas: default_gas_hashmap_to_safe_vec(domain_gas),
            default_gas,
            beneficiary: Some(relayer.address()),
        },
    );

    encode_call(relayer.private_key(), &call)
}

fn nonce() -> u64 {
    static NONCE: AtomicU64 = AtomicU64::new(0);
    NONCE.fetch_add(1, Ordering::Relaxed)
}

#![allow(dead_code, unused_imports, unused_variables)]
use crate::bank::helpers::*;
use crate::bank::{TOKEN_DECIMALS, TOKEN_NAME};
use crate::test_helpers::*;
use anyhow::Context;
use base64::prelude::BASE64_STANDARD;
use base64::Engine as _;
use demo_stf::runtime::Runtime;
use futures::StreamExt;
use serde::Deserialize;
use sov_api_spec::types::AcceptTxBody;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_full_node_configs::sequencer::{RecoveryStrategy, SequencerKindConfig};
use sov_mock_da::storable::StorableMockDaService;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{OperatingMode, Runtime as RuntimeTrait};
use sov_rollup_interface::node::ledger_api::IncludeChildren;
use sov_test_utils::test_rollup::{RollupBuilder, RollupProverConfig, TestRollup};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;
use std::sync::Arc;

/// Tests that chain hash overrides work as expected.
///
/// First, setup a rollup with an override that expires at rollup height 10.
/// Send two txs, one with the overriden chain hash and one with a standard chain hash. Only the overriden chain hash should be accepted.
/// Then, wait a while for the chain to pash the override window. Send the same two txs again (with the nonces updated). This time, only the standard chain hash should be accepted.
#[tokio::test(flavor = "multi_thread")]
async fn test_chain_hash_override() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES", "[{start_height = 0, end_height = 10, chain_hash = \"0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\"}]");
    sov_test_utils::logging::initialize_or_change_logging_with_filter("warn");
    let default_chain_hash = <Runtime<DemoRollupSpec> as RuntimeTrait<DemoRollupSpec>>::CHAIN_HASH;
    let test_rollup = start_test_rollup(
        &TestCase {
            wait_for_aggregated_proof: false,
            finalization_blocks: 0,
        },
        OperatingMode::Operator,
    )
    .await?;
    let client = test_rollup.client.client;

    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();
    // Send a tx with the standard chain hash. It should not be accepted because the override is active.
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        0,
        1000,
        Some(default_chain_hash),
        "First Token".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    assert!(
        client
            .accept_tx(&AcceptTxBody {
                body: BASE64_STANDARD.encode(&raw_tx),
            })
            .await
            .is_err(),
        "Should not be able to send tx with current chain hash until override expires"
    );

    // Send a tx with the overriden chain hash. It should be accepted.
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        0,
        1000,
        Some([255; 32]),
        "First Token".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    client
        .accept_tx(&AcceptTxBody {
            body: BASE64_STANDARD.encode(&raw_tx),
        })
        .await
        .unwrap();

    // Wait a while to pass rollup height 10 where the override expires.
    let mut slots_subscription = client
        .subscribe_slots_with_children(IncludeChildren::new(true))
        .await?;
    let mut done_waiting = false;
    for i in 0..50 {
        let slot = slots_subscription.next().await.unwrap()?;
        if slot.batch_range.start >= 10 {
            done_waiting = true;
            break;
        }
    }
    assert!(done_waiting, "Should have hit rollup height 10 within 30 slots. This is a bug in the test logic, not necessarily a bug in the rollup.");

    // Send a tx with the standard chain hash. It should be accepted because the override is no longer active.
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        1,
        1000,
        Some(default_chain_hash),
        "Second Token".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    client
        .accept_tx(&AcceptTxBody {
            body: BASE64_STANDARD.encode(&raw_tx),
        })
        .await
        .unwrap();

    // Send a tx with the old chain hash. It should *not* be accepted because the override is no longer active.
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        2,
        1000,
        Some([255; 32]),
        "Third Token".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    assert!(
        client
            .accept_tx(&AcceptTxBody {
                body: BASE64_STANDARD.encode(&raw_tx),
            })
            .await
            .is_err(),
        "Should not be able to send tx with old chain hash after override expires"
    );

    Ok(())
}

/// Tests that chain hash override grace periods work as expected.
///
/// Setup a rollup with an override that expires at rollup height 10 with a grace period of 5 blocks.
/// - Before height 10: only the override hash should be accepted
/// - During grace period (heights 10-14): both hashes should be accepted
/// - After grace period (height 15+): only the default hash should be accepted
#[tokio::test(flavor = "multi_thread")]
async fn test_chain_hash_override_grace_period() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES", "[{start_height = 0, end_height = 10, chain_hash = \"0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\", grace_period = 5}]");
    sov_test_utils::logging::initialize_or_change_logging_with_filter("warn");
    let default_chain_hash = <Runtime<DemoRollupSpec> as RuntimeTrait<DemoRollupSpec>>::CHAIN_HASH;
    let override_chain_hash = [255u8; 32];

    let test_rollup = start_test_rollup(
        &TestCase {
            wait_for_aggregated_proof: false,
            finalization_blocks: 0,
        },
        OperatingMode::Operator,
    )
    .await?;
    let client = test_rollup.client.client;

    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();

    // Phase 1: Before override expires (height < 10)
    // Only the override hash should be accepted

    // Send a tx with the default chain hash - should be rejected
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        0,
        1000,
        Some(default_chain_hash),
        "Token With Default Hash Before Expiry".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    assert!(
        client
            .accept_tx(&AcceptTxBody {
                body: BASE64_STANDARD.encode(&raw_tx),
            })
            .await
            .is_err(),
        "Before override expires: default chain hash should be rejected"
    );

    // Send a tx with the override chain hash - should be accepted
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        0,
        1000,
        Some(override_chain_hash),
        "Token With Override Hash Before Expiry".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    client
        .accept_tx(&AcceptTxBody {
            body: BASE64_STANDARD.encode(&raw_tx),
        })
        .await
        .expect("Before override expires: override chain hash should be accepted");

    // Wait for rollup height >= 10 (grace period starts)
    let mut slots_subscription = client
        .subscribe_slots_with_children(IncludeChildren::new(true))
        .await?;
    let mut current_height = 0;
    for _ in 0..50 {
        let slot = slots_subscription.next().await.unwrap()?;
        current_height = slot.batch_range.start;
        if current_height >= 10 {
            break;
        }
    }
    assert!(
        current_height >= 10,
        "Should have reached rollup height 10 for grace period"
    );

    // Phase 2: During grace period (10 <= height < 15)
    // Both hashes should be accepted

    // Send a tx with the default chain hash - should be accepted during grace period
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        1,
        1000,
        Some(default_chain_hash),
        "Token With Default Hash During Grace".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    client
        .accept_tx(&AcceptTxBody {
            body: BASE64_STANDARD.encode(&raw_tx),
        })
        .await
        .expect("During grace period: default chain hash should be accepted");

    // Send a tx with the override chain hash - should also be accepted during grace period
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        2,
        1000,
        Some(override_chain_hash),
        "Token With Override Hash During Grace".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    client
        .accept_tx(&AcceptTxBody {
            body: BASE64_STANDARD.encode(&raw_tx),
        })
        .await
        .expect("During grace period: override chain hash should also be accepted");

    // Wait for rollup height >= 15 (grace period ends)
    for _ in 0..50 {
        let slot = slots_subscription.next().await.unwrap()?;
        current_height = slot.batch_range.start;
        if current_height >= 15 {
            break;
        }
    }
    assert!(
        current_height >= 15,
        "Should have reached rollup height 15 (end of grace period)"
    );

    // Phase 3: After grace period (height >= 15)
    // Only the default hash should be accepted

    // Send a tx with the default chain hash - should be accepted
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        3,
        1000,
        Some(default_chain_hash),
        "Token With Default Hash After Grace".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    client
        .accept_tx(&AcceptTxBody {
            body: BASE64_STANDARD.encode(&raw_tx),
        })
        .await
        .expect("After grace period: default chain hash should be accepted");

    // Send a tx with the override chain hash - should be rejected
    let tx = build_create_token_tx_with_chain_hash_and_token_name(
        &key,
        4,
        1000,
        Some(override_chain_hash),
        "Token With Override Hash After Grace".to_string(),
    );
    let raw_tx = borsh::to_vec(&tx).unwrap();
    assert!(
        client
            .accept_tx(&AcceptTxBody {
                body: BASE64_STANDARD.encode(&raw_tx),
            })
            .await
            .is_err(),
        "After grace period: override chain hash should be rejected"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "fix when ZKP work is resumed again"]
async fn bank_tx_tests() -> anyhow::Result<()> {
    sov_test_utils::logging::initialize_or_change_logging_with_filter("warn");
    let test_case = TestCase {
        wait_for_aggregated_proof: true,
        finalization_blocks: 0,
    };

    let test_rollup = start_test_rollup(&test_case, OperatingMode::Optimistic).await?;

    // If the rollup throws an error, return it and stop trying to send the transaction
    tokio::select! {
        err = test_rollup.rollup_task => err?,
        res = send_test_bank_txs(test_case, &test_rollup.client, test_rollup.da_service.clone()) => Ok(res?),
    }?;

    Ok(())
}

async fn send_test_bank_txs(
    test_case: TestCase,
    client: &NodeClient,
    da_service: Arc<StorableMockDaService>,
) -> anyhow::Result<()> {
    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();
    let token_id_response = client
        .get_token_id::<DemoRollupSpec>(TOKEN_NAME, Some(TOKEN_DECIMALS), &user_address)
        .await?;

    const NUM_TRANSFERS: u64 = 5;

    assert_eq!(token_id, token_id_response);

    // create token.
    let initial_balance = 1000;
    let tx = build_create_token_tx(&key, 0, initial_balance);

    let mut slots_subscription = client.client.subscribe_slots().await?;
    let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;

    let mut processed_slot = slots_subscription.next().await.unwrap()?;
    while processed_slot.number < slot_number {
        processed_slot = slots_subscription.next().await.unwrap()?;
    }

    assert_slot_finality(client, slot_number, test_case.expected_head_finality()).await;
    assert_balance(client, initial_balance, token_id, user_address, None).await?;

    // Make a few transfers and check that attestation height progresses
    for nonce in 1..=NUM_TRANSFERS {
        let tx = build_transfer_token_tx(&key, token_id, recipient_address, 10, nonce);

        let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;

        assert_slot_finality(client, slot_number, test_case.expected_head_finality()).await;
        assert_balance(
            client,
            initial_balance - (nonce as u128) * 10,
            token_id,
            user_address,
            None,
        )
        .await?;

        tracing::info!(slot_number, "Tx has been published in slot");
        while processed_slot.number < slot_number {
            processed_slot = slots_subscription.next().await.unwrap()?;
        }
        tracing::info!("C {nonce} =========");
        tracing::info!("------------------------");

        let mut max_attested_height = get_max_attested_height(client).await?;
        tracing::info!(max_attested_height, "Starting max attested height");
        while max_attested_height < slot_number {
            // In some cases, the attestation may not be ready just yet. Let's try again in a bit.
            max_attested_height = get_max_attested_height(client)
                .await
                .with_context(|| format!("Final part of the test: failed to get max attested height for {slot_number}, only have {max_attested_height}"))?;
            tracing::info!(
                "KEEP WAITING: {nonce} =========: {max_attested_height} < {slot_number}"
            );
            slots_subscription.next().await.unwrap()?;
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct ValueResponse {
    value: u64,
}

async fn get_max_attested_height(client: &NodeClient) -> anyhow::Result<u64> {
    let response = client
        .query_rest_endpoint::<ValueResponse>(
            "/modules/attester-incentives/state/maximum-attested-height",
        )
        .await?;

    let height = response.value;
    Ok(height)
}

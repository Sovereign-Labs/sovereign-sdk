#![allow(dead_code, unused_imports, unused_variables)]
use std::str::FromStr;
use std::sync::Arc;

use serde::Deserialize;
use sov_address::MultiAddress;
use sov_bank::config_gas_token_id;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_da::storable::service::StorableMockDaService;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{OperatingMode, Spec};
use sov_test_utils::test_rollup::{RollupBuilder, RollupProverConfig};
use sov_test_utils::{TestSpec, TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING};

use crate::bank::helpers::*;
use crate::bank::{TOKEN_DECIMALS, TOKEN_NAME};
use crate::test_helpers::*;

#[tokio::test(flavor = "multi_thread")]
async fn flaky_bank_tx_tests_secured_by_operator() -> anyhow::Result<()> {
    // std::env::set_var("RUST_LOG", "info");
    // sov_test_utils::initialize_logging();
    let test_case = TestCase {
        wait_for_aggregated_proof: false,
        finalization_blocks: 0,
    };

    let test_rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Operator),
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        test_case.finalization_blocks,
    )
    .set_config(|c| c.max_concurrent_blobs = 65536)
    .disable_state_root_consistency_checks()
    .start()
    .await?;

    test_rollup
        .da_service
        .produce_n_blocks_now(3)
        .await
        .unwrap();

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

    let reward_addr = <TestSpec as Spec>::Address::from_str(
        "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
    )
    .unwrap();

    {
        let reward_amount = client
            .get_balance::<TestSpec>(&reward_addr, &config_gas_token_id(), None)
            .await;
        assert!(reward_amount.is_err());
    }

    const NUM_TRANSFERS: u64 = 5;

    assert_eq!(token_id, token_id_response);

    // create token.
    let initial_balance = 1000;
    let tx = build_create_token_tx(&key, 0, initial_balance);

    let slot_number = send_tx_and_wait_for_status(&[tx], client).await?;

    // Will cause a batch to be produced.
    da_service.produce_n_blocks_now(1).await.unwrap();

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

        da_service.produce_n_blocks_now(1).await.unwrap();
    }

    {
        let reward_amount = client
            .get_balance::<TestSpec>(&reward_addr, &config_gas_token_id(), None)
            .await
            .unwrap();
        assert!(reward_amount > 0);
    }

    Ok(())
}

use anyhow::Context;
use futures::StreamExt;
use sov_api_spec::types::AggregatedProof as ApiAggregatedProof;
use sov_bank::event::Event as BankEvent;
use sov_bank::utils::TokenHolder;
use sov_bank::Coins;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_zkvm::{MockCodeCommitment, MockZkVerifier};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{Amount, OperatingMode, SerializedAggregatedProof, Spec};
use sov_rollup_interface::node::ledger_api::FinalityStatus;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregateProofVerifier, AggregatedProofPublicData,
};
use sov_sequencer::SequencerKindConfig;
use sov_state::Storage;
use sov_test_utils::test_rollup::{RollupBuilder, RollupProverConfig};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::bank::helpers::*;
use crate::bank::{TOKEN_DECIMALS, TOKEN_NAME};
use crate::test_helpers::{test_genesis_source, DemoRollupSpec};

type TestSpec = DemoRollupSpec;

const WAIT_TIME: u64 = 500;

#[tokio::test(flavor = "multi_thread")]
async fn flaky_bank_tx_tests_periodic_da_instant_finality() -> anyhow::Result<()> {
    inner(0).await
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_bank_tx_tests_periodic_da_non_instant_finality() -> anyhow::Result<()> {
    inner(2).await
}

async fn inner(finalization_blocks: u32) -> anyhow::Result<()> {
    let test_case: TestCase = TestCase {
        wait_for_aggregated_proof: true,
        finalization_blocks,
    };

    let test_rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Zk),
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        test_case.finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.rollup_prover_config = Some(RollupProverConfig::Skip);
        // Since we've enabled the prover, we need to disable the state root consistency checks
        // This is because proofs are not yet played in the sequencer, causing the state root to be incorrect
        if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
            sequencer_conf.disable_state_root_consistency_checks = true;
        }
    })
    .start()
    .await?;

    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // If the rollup throws an error, return it and stop trying to send the transaction
    tokio::select! {
        err = test_rollup.rollup_task => err?,
        res = send_test_bank_txs(test_case, &test_rollup.client) => Ok(res?),
    }?;

    Ok(())
}

async fn send_test_bank_txs(test_case: TestCase, client: &NodeClient) -> anyhow::Result<()> {
    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();

    let genesis_gas_balance = client
        .get_balance::<TestSpec>(&user_address, &sov_bank::config_gas_token_id(), Some(0))
        .await?;

    // There's no guarantee that we subscribed before the first proof is published.
    // But we know that it should be less or equal rollup_height of the first published batch
    let mut aggregated_proof_subscription = client
        .client
        .subscribe_aggregated_proof()
        .await
        .context("Failed to subscribe to aggregated proof")?;

    let token_id_response = client
        .get_token_id::<TestSpec>(TOKEN_NAME, Some(TOKEN_DECIMALS), &user_address)
        .await?;

    assert_eq!(token_id, token_id_response);

    let tx = build_create_token_tx(&key, 0, 1000);

    let _slot_batch_1 = send_tx_and_wait_for_status(&[tx], client).await?;

    assert_balance(client, 1000, token_id, user_address, None)
        .await
        .context("Initial balance at latest version")?;

    tokio::time::sleep(std::time::Duration::from_millis(WAIT_TIME)).await;
    // transfer 100 tokens. assert sender balance.
    let tx = build_transfer_token_tx(&key, token_id, recipient_address, 100, 1);
    let _slot_batch_2 = send_tx_and_wait_for_status(&[tx], client).await?;

    assert_balance(client, 900, token_id, user_address, None)
        .await
        .context("Balance decreased after first transaction, latest version")?;

    tokio::time::sleep(std::time::Duration::from_millis(WAIT_TIME)).await;

    let gas_balance_height_1 = client
        .get_balance::<TestSpec>(&user_address, &sov_bank::config_gas_token_id(), None)
        .await?;

    assert!(gas_balance_height_1 < genesis_gas_balance);

    // transfer 200 tokens. assert sender balance.
    let tx = build_transfer_token_tx(&key, token_id, recipient_address, 200, 2);

    let _slot_batch_3 = send_tx_and_wait_for_status(&[tx], client).await?;

    assert_balance(client, 700, token_id, user_address, None)
        .await
        .context("Balance decreased after second transaction, latest version")?;

    tokio::time::sleep(std::time::Duration::from_millis(WAIT_TIME)).await;

    // 10 transfers of 10,11..20
    let transfer_amounts: Vec<u128> = (10u128..20).collect();
    let txs = build_multiple_transfers(&transfer_amounts, &key, token_id, recipient_address, 3);
    let slot_batch_n = send_tx_and_wait_for_status(&txs, client).await?;
    assert_slot_finality(client, slot_batch_n, test_case.expected_head_finality()).await;

    // FIXME(@neysofu,
    // https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2341): these
    // tests use slot numbers as rollup heights. This is not correct.
    // // Test historical balance
    // assert_balance(client, 1000, token_id, user_address, Some(slot_batch_1)).await?;
    // assert_balance(client, 900, token_id, user_address, Some(slot_batch_2)).await?;
    // assert_balance(client, 700, token_id, user_address, Some(slot_batch_3)).await?;

    assert_bank_event::<TestSpec>(
        client,
        0,
        BankEvent::TokenCreated {
            token_name: TOKEN_NAME.to_owned(),
            coins: Coins {
                amount: Amount::new(1000),
                token_id,
            },
            minter: TokenHolder::User(user_address),
            mint_to_address: TokenHolder::User(user_address),
            admins: vec![],
            supply_cap: Amount::MAX,
        },
    )
    .await?;

    assert_bank_event::<TestSpec>(
        client,
        1,
        BankEvent::TokenTransferred {
            from: TokenHolder::User(user_address),
            to: TokenHolder::User(recipient_address),
            coins: Coins {
                amount: Amount::new(100),
                token_id,
            },
        },
    )
    .await?;

    assert_bank_event::<TestSpec>(
        client,
        2,
        BankEvent::TokenTransferred {
            from: TokenHolder::User(user_address),
            to: TokenHolder::User(recipient_address),
            coins: Coins {
                amount: Amount::new(200),
                token_id,
            },
        },
    )
    .await?;

    if test_case.wait_for_aggregated_proof {
        let aggregated_proof_resp: ApiAggregatedProof =
            aggregated_proof_subscription.next().await.unwrap().unwrap();

        let proof: SerializedAggregatedProof = aggregated_proof_resp.try_into()?;
        let verifier = AggregateProofVerifier::<MockZkVerifier>::new(MockCodeCommitment::default());
        let _pub_data: AggregatedProofPublicData<
            <TestSpec as Spec>::Address,
            <TestSpec as Spec>::Da,
            <<TestSpec as Spec>::Storage as Storage>::Root,
        > = verifier.verify(&proof)?;

        // FIXME(@neysofu,
        // https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2341):
        // relates to the FIXME above (misuse of slot numbers
        // as rollup heights).
        // assert!(slot_batch_1 >= pub_data.initial_slot_number.get());
        // assert!(slot_batch_1 >= pub_data.final_slot_number.get());

        // We can only check this under periodic block producing.
        // More thorough checks should be done in "OnSubmit" batch producing
        assert_aggregated_proof(1, 1, client).await?;
    }

    if let Some(finalized_rollup_height) = test_case.get_latest_finalized_slot_after(slot_batch_n) {
        assert_slot_finality(client, finalized_rollup_height, FinalityStatus::Finalized).await;
    }

    Ok(())
}

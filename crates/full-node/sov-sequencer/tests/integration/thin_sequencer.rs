#![allow(dead_code)] // FIXME(@neysofu): remove this once the test is fixed.

use std::sync::Arc;

use anyhow::Context;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use futures::StreamExt;
use sov_api_spec::types::AcceptTxBody;
use sov_blob_storage::config_deferred_slots_count;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockBlock};
use sov_modules_api::{Amount, RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::common::SafeVec;
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::TxHash;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder};
use sov_test_utils::{
    default_test_signed_transaction, generate_optimistic_runtime, RtAgnosticBlueprint, TestSpec,
    TestUser,
};

generate_optimistic_runtime!(TestRuntime <=);

type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

//#[tokio::test(flavor = "multi_thread")]
async fn test_thin_direct_same_transactions() {
    // Test starts a rollup and thin direct sequencer.
    // It submits the same transactions to both and checks that:
    //  1. Thin sequencer returns the same tx_hashes-the
    //  2. The same blob is posted to DA.
    let dir1 = Arc::new(tempfile::tempdir().unwrap());

    let genesis_config: HighLevelOptimisticGenesisConfig<TestSpec> =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let genesis_conf_seq_da_address = genesis_config.initial_sequencer.da_address;
    let mut genesis_params = GenesisParams {
        runtime: <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.clone().into(),
        ),
    };
    genesis_params
        .runtime
        .sequencer_registry
        .sequencer_config
        .is_preferred_sequencer = false;

    let test_rollup = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        1,
    )
    .set_config(|c| {
        c.storage = dir1;
        c.rollup_prover_config = None;
    })
    .set_da_config(|c| {
        c.sender_address = genesis_conf_seq_da_address;
    })
    .with_standard_sequencer()
    .with_secondary_sequencer(MockAddress::new([128; 32]))
    .start()
    .await
    .unwrap();

    let test_sequencer_client = test_rollup
        .secondary_test_sequencer_client
        .as_ref()
        .unwrap();

    let head = test_rollup
        .da_service
        .get_head_block_header()
        .await
        .unwrap()
        .height;
    let mut slots = test_rollup.api_client().subscribe_slots().await.unwrap();

    let user = genesis_config.additional_accounts().first().unwrap();
    // TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/947
    //    Ideally we want to test multiple transactions, but this bug prevents it
    // let all_txs = generate_txs(user, 10);
    // for tx in &all_txs {
    //     let tx_hash_accepted_a = accept_tx_in_rollup(&test_rollup.api_client(), &tx).await?;
    //     let tx_hash_accepted_b = accept_tx_in_rollup(test_sequencer_client, &tx).await?;
    //     assert_eq!(tx_hash_accepted_a, tx_hash_accepted_b);
    // }

    let tx = generate_tx_with_nonce(user, 1);
    let tx_hash_accepted_a = accept_tx_in_rollup(test_rollup.api_client(), &tx)
        .await
        .unwrap();
    let tx_hash_accepted_b = accept_tx_in_rollup(test_sequencer_client, &tx)
        .await
        .unwrap();
    assert_eq!(tx_hash_accepted_a, tx_hash_accepted_b);

    let deferred_slots_count = config_deferred_slots_count();
    let mut height_to_check = head + 1;
    for i in 1..=deferred_slots_count {
        test_rollup.da_service.produce_block_now().await.unwrap();
        let block = test_rollup.da_service.get_block_at(head + i).await.unwrap();
        if !block.batch_blobs.is_empty() {
            height_to_check = head + i;
            test_rollup.da_service.produce_block_now().await.unwrap();
            break;
        }
    }
    // Wait for the slot to be processed, so rollup is in a good state.
    let _slot = slots.next().await.unwrap().unwrap();
    compare_block_at_height(height_to_check, &test_rollup.da_service).await;
}

fn generate_tx_with_nonce(user: &TestUser<TestSpec>, nonce: u64) -> RawTx {
    let msg = TestRuntimeCall::Bank(
        sov_test_utils::sov_bank::CallMessage::<TestSpec>::CreateToken {
            token_name: format!("sequencers-check-{nonce}").try_into().unwrap(),
            token_decimals: None,
            initial_balance: Amount::new(1000),
            mint_to_address: user.address(),
            admins: SafeVec::new(),
            supply_cap: None,
        },
    );

    let tx = default_test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        &user.private_key,
        &msg,
        nonce,
        &TestRuntime::<TestSpec>::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

async fn accept_tx_in_rollup(
    api_client: &sov_api_spec::client::Client,
    tx: &RawTx,
) -> anyhow::Result<TxHash> {
    let accept_tx_body = AcceptTxBody {
        body: BASE64_STANDARD.encode(&tx.data),
    };
    let tx_accepted = api_client.accept_tx(&accept_tx_body).await?;
    tx_accepted.id.parse()
}

async fn compare_block_at_height(height: u64, da_service: &StorableMockDaService) {
    let block_1 = da_service
        .get_block_at(height)
        .await
        .expect("Failed to get block from DaService1");
    assert_eq!(
        block_1.batch_blobs.len(),
        1,
        "standard sequencer did not produce a batch"
    );
    let blob_from_std = get_single_blob(block_1).context("std sequencer").unwrap();
    let block_2 = da_service.get_block_at(height + 1).await.unwrap();
    let blob_from_stateless = get_single_blob(block_2)
        .context("stateless sequencer")
        .unwrap();
    assert!(!blob_from_std.is_empty());
    assert_eq!(
        blob_from_std, blob_from_stateless,
        "different blobs at height from sequencers"
    );
}

fn get_single_blob(mut block: MockBlock) -> anyhow::Result<Vec<u8>> {
    if block.batch_blobs.len() != 1 {
        anyhow::bail!(
            "Block does not have a single blob, but {}",
            block.batch_blobs.len()
        );
    }
    Ok(block
        .batch_blobs
        .iter_mut()
        .map(|b| b.full_data().to_vec())
        .next()
        .unwrap())
}

use std::str::FromStr;

use base64::prelude::*;
use borsh::BorshDeserialize;
use sov_api_spec::types;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_modules_api::prelude::*;
use sov_modules_api::{Address, BlobReaderTrait, DispatchCall, FullyBakedTx};
use sov_rollup_interface::node::da::DaService;
use sov_sequencer::standard::StdSequencerConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{config_gas_token_id, Coins, TestOptimisticRuntime};
use sov_test_utils::sequencer::TestSequencerSetup;
use sov_test_utils::{TestSpec, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_USER_BALANCE};

use crate::utils::{
    build_tx, generate_paymaster_tx, new_sequencer, valid_tx_bytes, wrap_with_auth, RT,
};

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_happy_path() {
    let sequencer = new_sequencer().await;
    let tx1 = valid_tx_bytes(&sequencer, 0, 0);
    let tx2 = valid_tx_bytes(&sequencer, 1, 1);
    sequencer
        .client()
        .accept_tx(&types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx1),
        })
        .await
        .unwrap();

    sequencer
        .client()
        .accept_tx(&types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx2),
        })
        .await
        .unwrap();
    let _ = sequencer.sequencer.produce_and_submit_batch().await;

    let mut submitted_block = sequencer.da_service.get_block_at(1).await.unwrap();
    let block_data = submitted_block.batch_blobs[0].full_data();

    let batch = Vec::<FullyBakedTx>::try_from_slice(block_data).unwrap();

    assert_eq!(batch.len(), 2);
    assert_eq!(wrap_with_auth(tx1), batch[0]);
    assert_eq!(wrap_with_auth(tx2), batch[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_accept_tx() {
    let sequencer = new_sequencer().await;

    let client = sequencer.client();

    let tx = valid_tx_bytes(&sequencer, 0, 0);

    client
        .accept_tx(&types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx.data),
        })
        .await
        .unwrap();
    let _ = sequencer.sequencer.produce_and_submit_batch().await;

    let mut submitted_block = sequencer.da_service.get_block_at(1).await.unwrap();
    let block_data = submitted_block.batch_blobs[0].full_data();
    let batch = Vec::<FullyBakedTx>::try_from_slice(block_data).unwrap();

    assert_eq!(wrap_with_auth(tx).data, batch[0].data);
}

#[tokio::test(flavor = "multi_thread")]
// Test how the batch builder handles invalid transactions inside the mempool by
// inserting two valid transactions, then draining the sender's balance so that
// the second one has insufficient gas. This is a regression test for the bug
// where an out-of-gas error during batch creation prevented the entire
// batch from being submitted.
async fn test_batch_building_with_out_of_gas_error() {
    let sequencer = new_sequencer().await;

    // -- Build a transaction which drains the user's wallet --
    let drain_wallet_msg = sov_test_utils::runtime::BankCallMessage::<TestSpec>::Transfer {
        to: Address::from_str("sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv").unwrap(),
        coins: Coins {
            amount: TEST_DEFAULT_USER_BALANCE
                .checked_sub(TEST_DEFAULT_MAX_FEE)
                .unwrap(),
            // Leave enough tokens to pay gas for the first tx
            token_id: config_gas_token_id(),
        },
    };
    let drain_wallet_msg =
        <TestOptimisticRuntime<TestSpec> as DispatchCall>::Decodable::Bank(drain_wallet_msg);
    // --  END tx construction --

    let drainer = build_tx(&sequencer, 0, &drain_wallet_msg);
    let tx_with_insufficient_gas = valid_tx_bytes(&sequencer, 1, 1);

    // Send the two transactions, drainer first. Since the default builder
    // uses FIFO, this ensures that it gets included first, leaving the other one out of gas.
    let client = sequencer.client();
    client
        .accept_tx(&types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&drainer.data),
        })
        .await
        .unwrap();

    client
        .accept_tx(&types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_with_insufficient_gas.data),
        })
        .await
        .unwrap();
    let _ = sequencer.sequencer.produce_and_submit_batch().await;

    // As a sanity check, assert that the second transaction wasn't included
    let mut submitted_block = sequencer.da_service.get_block_at(1).await.unwrap();
    let block_data = submitted_block.batch_blobs[0].full_data();
    let batch = Vec::<FullyBakedTx>::try_from_slice(block_data).unwrap();
    assert_eq!(wrap_with_auth(drainer).data, batch[0].data);
    assert_eq!(batch.len(), 1);
}

// Checks that transactions that are not sequencer safe are rejected
// when the sender address is not configured as an admin in the sequencer config.
#[tokio::test(flavor = "multi_thread")]
async fn not_sequencer_safe_txs_are_restricted() {
    let dir = tempfile::tempdir().unwrap();
    let sequencer_addr = HighLevelOptimisticGenesisConfig::<TestSpec>::sequencer_da_addr();
    let da_service = StorableMockDaService::new_in_memory(sequencer_addr, 0).await;
    let sequencer_config = StdSequencerConfig {
        mempool_max_txs_count: None,
        max_batch_size_bytes: None,
    };

    let sequencer = TestSequencerSetup::<RT>::new(dir, da_service, sequencer_config, false)
        .await
        .unwrap();

    let tx = generate_paymaster_tx::<TestOptimisticRuntime<TestSpec>>(
        sequencer.admin_private_key.clone(),
    );
    {
        let client = sequencer.client();

        client
            .accept_tx(&types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        assert!(
            sequencer
                .sequencer
                .produce_and_submit_batch()
                .await
                .is_none(),
            "Sequencer accepted admin tx from non-admin sender"
        );
    }
}

// Checks that transactions that are not sequencer safe are accepted
// if the sender address is configured as an admin in the sequencer config.
#[tokio::test(flavor = "multi_thread")]
async fn sequencer_safe_txs_from_admins_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let sequencer_addr = HighLevelOptimisticGenesisConfig::<TestSpec>::sequencer_da_addr();
    let da_service = StorableMockDaService::new_in_memory(sequencer_addr, 0).await;
    let sequencer_config = StdSequencerConfig {
        mempool_max_txs_count: None,
        max_batch_size_bytes: None,
    };

    let sequencer = TestSequencerSetup::<RT>::new(dir, da_service, sequencer_config, true)
        .await
        .unwrap();

    let tx = generate_paymaster_tx::<TestOptimisticRuntime<TestSpec>>(
        sequencer.admin_private_key.clone(),
    );
    {
        let client = sequencer.client();

        client
            .accept_tx(&types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        assert!(
            sequencer
                .sequencer
                .produce_and_submit_batch()
                .await
                .is_some(),
            "Batch publication should succeed because the provided tx is valid and comes from an admin"
        );
    }
}

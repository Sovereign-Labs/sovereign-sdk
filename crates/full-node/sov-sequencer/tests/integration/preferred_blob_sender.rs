use std::env;
use std::time::Duration;

use futures::StreamExt;
use sov_blob_sender::BlobSelectorStatus;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::prelude::*;
use sov_modules_api::{DispatchCall, RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::PaymasterConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::{
    default_test_signed_transaction, TestSpec, TestUser, TEST_BLOB_PROCESSING_TIMEOUT,
    TEST_MAX_BATCH_SIZE,
};
use sov_value_setter::ValueSetterConfig;

#[allow(unused_imports)]
use crate::preferred_end_to_end::{
    run_action_against_test_rollup, run_actions_against_test_rollup,
    setup_test_rollup_with_initial_state, InvalidGeneration, TestBlueprint, TestRuntime, TestState,
    TestingAction,
};
use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, MAX_BATCH_EXECUTION_TIME_MILLIS};

async fn create_test_rollup() -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
            (),
            PaymasterConfig::default(),
            (),
        );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = tempdir_inside_codebase_dir();

    (
        new_test_rollup::<TestRuntime<TestSpec>>(
            dir,
            genesis_params
                .runtime
                .sequencer_registry
                .sequencer_config
                .seq_da_address,
            genesis_params,
            0,
            true,
            TEST_MAX_BATCH_SIZE,
            BlockProducingConfig::Periodic { block_time_ms: 300 },
            //BlockProducingConfig::Manual,
            None,
            TEST_BLOB_PROCESSING_TIMEOUT,
            1,
            MAX_BATCH_EXECUTION_TIME_MILLIS,
            None,
            0,
        )
        .await
        .map(|v| v.into_iter().next().unwrap())
        .unwrap(),
        admin,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn test_discard_oversized_blobs() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "1000",
    );
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.da_service.produce_block_now().await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let client = test_rollup.api_client.clone();

    // Blob with this transaction will be discarded becuse the blob is bigger than `MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE`
    let tx = tx_set_many_values(&admin.private_key, 0, vec![7; 10000]);
    let _ = client.send_raw_tx_to_sequencer(&tx).await.unwrap();
    let mut sub = test_rollup
        .subscribe_to_blobs_from_blob_sender()
        .await
        .unwrap();

    tokio::time::timeout(tokio::time::Duration::from_secs(15), async {
        while let Some(blob_status) = sub.next().await {
            if let Some(BlobSelectorStatus::Discarded) =
                blob_status.as_ref().unwrap().blob_selector_status
            {
                break;
            }
        }
    })
    .await
    .expect("Timeout occurred while waiting for the discarded blob.");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_blobs_are_send_after_rollup_resync() {
    let (test_rollup, _) = create_test_rollup().await;
    let da = test_rollup.da_service.clone();
    let mut header_subscrition = da.subscribe_finalized_header().await.unwrap();

    for _ in 0..10 {
        da.produce_block_now().await.unwrap();
        header_subscrition.next().await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let builder = test_rollup.shutdown().await.unwrap();

    // Generate a block while Rollup is offline to trigger resync logic.
    for _ in 0..20 {
        da.produce_block_now().await.unwrap();
        header_subscrition.next().await.unwrap().unwrap();
    }

    // The new rollup has pending blobs in the BlobSender DB and completed blobs in the Preferred Sequencer state.
    let test_rollup = builder.start().await.unwrap();
    let mut subscribe_state_updates = test_rollup.subscribe_state_updates().await.unwrap();
    let mut subscribe_to_blobs_from_blob_sender = test_rollup
        .subscribe_to_blobs_from_blob_sender()
        .await
        .unwrap();

    // BlobSender should send blobs only after resync is complete, so the subscribe_state_updates notification must come first.
    tokio::select! {
        _ = subscribe_state_updates.next() => {}
        _ = subscribe_to_blobs_from_blob_sender.next() => {
            panic!("In a resync scenario, the state update notification should occur before the blob sender transmits the blobs.")
        }
    }
}

fn tx_set_many_values(key: &Ed25519PrivateKey, nonce: u64, values_to_set: Vec<u8>) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetManyValues(values_to_set),
    );
    encode_call(key, nonce, &msg)
}

fn encode_call(
    key: &Ed25519PrivateKey,
    nonce: u64,
    call_message: &<TestRuntime<TestSpec> as DispatchCall>::Decodable,
) -> RawTx {
    let tx = default_test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        call_message,
        nonce,
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

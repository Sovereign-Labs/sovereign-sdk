use crate::utils::{new_test_rollup, MAX_BATCH_EXECUTION_TIME_MILLIS};
use futures::StreamExt;
use sov_kernels::soft_confirmations::SoftConfirmationsKernel;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::prelude::*;
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_api::{Amount, EncodeCall, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_test_utils::runtime::genesis::operator::HighLevelOperatorGenesisConfig;
use sov_test_utils::runtime::Bank;
use sov_test_utils::sov_bank::{config_gas_token_id, CallMessage as BankCallMessage, Coins};
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::{
    default_test_tx_details, generate_operator_runtime_with_kernel, RtAgnosticBlueprint, TestSpec,
    TestUser, TEST_BLOB_PROCESSING_TIMEOUT, TEST_DEFAULT_USER_BALANCE, TEST_MAX_BATCH_SIZE,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use std::sync::Arc;

generate_operator_runtime_with_kernel!(kernel_type: SoftConfirmationsKernel<'a, S>, TestRuntime <= value_setter: ValueSetter<S>);
type RT = TestRuntime<TestSpec>;
type TestBlueprint = RtAgnosticBlueprint<TestSpec, RT>;

#[allow(clippy::too_many_arguments)]
async fn create_test_rollup() -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let reward_user = TestUser::<TestSpec>::generate(TEST_DEFAULT_USER_BALANCE);

    let genesis_config =
        HighLevelOperatorGenesisConfig::<TestSpec>::generate_with_additional_accounts(
            2,
            reward_user,
        );

    let admin = genesis_config.additional_accounts()[0].clone();
    let rt_genesis_config = <RT as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
    );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = Arc::new(tempfile::tempdir().unwrap());

    (
        new_test_rollup::<RT>(
            dir.clone(),
            genesis_params
                .runtime
                .sequencer_registry
                .sequencer_config
                .seq_da_address,
            genesis_params,
            0,
            true,
            TEST_MAX_BATCH_SIZE,
            // TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
            BlockProducingConfig::Manual,
            None,
            TEST_BLOB_PROCESSING_TIMEOUT,
            1,
            MAX_BATCH_EXECUTION_TIME_MILLIS,
            None,
            1,
        )
        .await
        .map(|v| v.into_iter().next().unwrap())
        .unwrap(),
        admin,
    )
}

/// Test demonstrating how nonce and generation can be used independently
/// This test shows that:
/// 1. Nonces and generations are tracked separately per account
/// 2. You can skip values in both sequences
/// 3. You cannot reuse a nonce or generation that was already consumed
#[tokio::test(flavor = "multi_thread")]
async fn test_mixed_nonce_and_generation_transactions() {
    sov_test_utils::logging::initialize_or_change_logging_with_filter(
        "warn,sov_metrics=error,sov=debug,integration=debug",
    );

    tracing::info!("STARTING ROLLUP");
    let (test_rollup, test_user) = create_test_rollup().await;
    tracing::info!("ROLLUP STARTED");
    test_rollup
        .da_service
        .produce_n_blocks_now(3)
        .await
        .unwrap();
    tracing::info!("PRODUCED SOME BLOCKS");

    let client = test_rollup.client.client.clone();

    let mut finalized_slots = client.subscribe_finalized_slots().await.unwrap();
    let x = finalized_slots.next().await;
    tracing::info!("X1: {:?}", x);
    let x = finalized_slots.next().await;
    tracing::info!("X2: {:?}", x);

    // Create a simple burn message for all transactions
    let msg = <RT as EncodeCall<Bank<TestSpec>>>::to_decodable(BankCallMessage::Burn {
        coins: Coins {
            amount: Amount::new(3),
            token_id: config_gas_token_id(),
        },
    });
    let details = default_test_tx_details();

    let construct_tx = |uniqueness: UniquenessData| {
        let unsigned_tx =
            UnsignedTransaction::new_with_details(msg.clone(), uniqueness, details.clone());
        Transaction::<RT, TestSpec>::new_signed_tx(
            &test_user.private_key,
            &RT::CHAIN_HASH,
            unsigned_tx,
        )
    };

    // Test sequence:
    // // 1. Submit tx with generation 1 -> should succeed
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(0)))
    //     .await;
    // tracing::info!("Result 1: {:?}", result);
    // assert!(result.is_ok(), "Generation 1 should succeed");
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;

    // 2. Submit tx with nonce 1 -> should succeed (independent counter)
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(0)))
        .await;
    tracing::info!("Result 2: {:?}", result);
    assert!(result.is_ok(), "Nonce 1 should succeed");
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;

    // TESTING
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(2)))
        .await;
    tracing::info!("RESULT NONCE 2: {:?}", result);
    assert!(
        result.is_ok(),
        "Nonce 2 should succeed (skipping is allowed)"
    );
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;

    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(1)))
        .await;
    tracing::info!("RESULT NONCE 1: {:?}", result);
    assert!(result.is_err(), "Nonce 1 should fail");
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;

    //  END OF TESTING

    // // 3. Submit tx with generation 2 -> should succeed
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(1)))
    //     .await;
    // tracing::info!("Result 3: {:?}", result);
    // assert!(result.is_ok(), "Generation 2 should succeed");
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;

    // // 4. Submit tx with generation 10 -> should succeed (can skip values)
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(10)))
    //     .await;
    // tracing::info!("Result 4: {:?}", result);
    // assert!(
    //     result.is_ok(),
    //     "Generation 10 should succeed (skipping is allowed)"
    // );
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;
    //
    // // 5. Submit tx with nonce 9 -> should succeed (can skip values)
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(9)))
    //     .await;
    // assert!(
    //     result.is_ok(),
    //     "Nonce 9 should succeed (skipping is allowed)"
    // );
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;
    //
    // // 6. Submit tx with nonce 15 -> should succeed
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(15)))
    //     .await;
    // assert!(result.is_ok(), "Nonce 15 should succeed");
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;
    //
    // // 7. Submit tx with generation 13 -> should succeed
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(13)))
    //     .await;
    // assert!(result.is_ok(), "Generation 13 should succeed");
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;
    //
    // // 8. Submit tx with nonce 14 -> should fail (14 < 15, already used higher)
    // tracing::info!("Submitting tx with nonce 14 (should fail)...");
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(14)))
    //     .await;
    // tracing::info!("RESULT: {:?}", result);
    // assert!(
    //     result.is_err(),
    //     "Nonce 14 should fail - already used nonce 15"
    // );
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;
    //
    // // 9. Submit tx with generation 12 -> should fail (12 < 13, already used higher)
    // println!("Submitting tx with generation 12 (should fail)...");
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(12)))
    //     .await;
    // assert!(
    //     result.is_err(),
    //     "Generation 12 should fail - already used generation 13"
    // );
    //
    // // 10. Submit tx with generation 1 again -> should fail (already used)
    // println!("Submitting tx with generation 1 again (should fail)...");
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(1)))
    //     .await;
    // assert!(result.is_err(), "Generation 1 should fail - already used");
    //
    // // 11. Submit tx with nonce 1 again -> should fail (already used)
    // println!("Submitting tx with nonce 1 again (should fail)...");
    // let result = client
    //     .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(1)))
    //     .await;
    // assert!(result.is_err(), "Nonce 1 should fail - already used");
    //
    // // Produce a batch to ensure transactions are processed
    // let _ = sequencer.sequencer.produce_and_submit_batch().await;

    println!("Test completed successfully!");
}

use crate::utils::{new_test_rollup, MAX_BATCH_EXECUTION_TIME_MILLIS};
use futures::StreamExt;
use sov_kernels::soft_confirmations::SoftConfirmationsKernel;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::prelude::*;
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_api::{Amount, EncodeCall, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::crypto::{PrivateKey, PublicKey};
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

async fn create_test_rollup() -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let reward_user = TestUser::<TestSpec>::generate(TEST_DEFAULT_USER_BALANCE);

    let genesis_config =
        HighLevelOperatorGenesisConfig::<TestSpec>::generate_with_additional_accounts(
            1,
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

/// Test demonstrating how nonce and generation can be used independently.
/// This test shows that:
/// 1. Nonces and generations are tracked separately per account
/// 2. Generations allow skipping values (e.g., 0 -> 3 -> 6)
/// 3. Nonces must be sequential (0 -> 1 -> 2...), skipping is not allowed
/// 4. You cannot reuse a nonce or generation that was already consumed
/// 5. Both mechanisms can be used interchangeably for the same account
#[tokio::test(flavor = "multi_thread")]
async fn test_mixed_nonce_and_generation_transactions() {
    // Keep it commented out in case of debug.
    sov_test_utils::logging::initialize_or_change_logging_with_filter(
        "warn,sov_metrics=error,sov=debug,integration=debug",
    );

    let (test_rollup, test_user) = create_test_rollup().await;
    test_rollup
        .da_service
        .produce_n_blocks_now(3)
        .await
        .unwrap();

    let client = test_rollup.client.client.clone();

    // let addr1 = test_user.address();
    let pub_key_hex = hex::encode(test_user.private_key.pub_key().bytes());
    tracing::info!("PUB KEY HEX: {}", pub_key_hex);
    let credential_id = test_user.private_key.pub_key().credential_id();
    let default_nonce = client.get_next_nonce(&credential_id).await.unwrap();
    assert_eq!(0, default_nonce);

    let mut finalized_slots = client.subscribe_finalized_slots().await.unwrap();
    let _ = finalized_slots.next().await;
    let _ = finalized_slots.next().await;

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
    // 1. Submit tx with generation 0 -> should succeed
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(0)))
        .await;
    assert!(result.is_ok(), "Generation 0 should succeed");

    // 2. Submit tx with nonce 0 -> should succeed (independent counter)
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(0)))
        .await;
    assert!(result.is_ok(), "Nonce 0 should succeed");

    let next_available = client.get_next_nonce(&credential_id).await.unwrap();
    assert_eq!(1, next_available);

    // 3. Submit tx with nonce 2 -> should fail, skipping is not allowed for nonces
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(2)))
        .await;
    assert!(
        result.is_err(),
        "Nonce 2 should fail (skipping is not allowed)"
    );

    // Submit nonces 1, 2, 3, 4 in sequence (must be sequential)
    for nonce in 1..=4 {
        let result = client
            .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(nonce)))
            .await;
        assert!(result.is_ok(), "Nonce {nonce} should succeed");
    }
    // Last nonce used is 4, next expected nonce is 5

    // 4. Submit tx with generation 3 (skipping 1 and 2) -> should succeed
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(3)))
        .await;
    assert!(result.is_ok(), "Generation 3 should succeed");

    // 5. Submit tx with generation 6 (skipping 4 and 5) -> should succeed
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(6)))
        .await;
    assert!(result.is_ok(), "Generation 6 should succeed");

    let last_nonce = client.get_next_nonce(&credential_id).await.unwrap();
    assert_eq!(5, last_nonce);

    // 6. Submit tx with nonce 5 -> should succeed (continues from nonce 4, independent of generation)
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Nonce(5)))
        .await;
    assert!(result.is_ok(), "Nonce 5 should succeed");

    let last_nonce = client.get_next_nonce(&credential_id).await.unwrap();
    assert_eq!(6, last_nonce);

    // 7. Submit tx with generation 6 again -> should fail (already used)
    let result = client
        .send_tx_to_sequencer(&construct_tx(UniquenessData::Generation(6)))
        .await;
    assert!(result.is_err(), "Generation 6 should fail (already used)");
}

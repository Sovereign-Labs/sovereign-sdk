use std::collections::BTreeSet;
use std::time::Duration;

use borsh::to_vec;
use sov_api_spec::types::TxReceiptResult;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::{EncodeCall, FullyBakedTx, HDTimestamp, RawTx, Runtime, SequencingData};
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::node::da::DaService;
use sov_test_modules::sequencing_data::{CallMessage, SequencingDataTester};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::{
    default_test_signed_transaction, generate_runtime, TestSpec, TestUser,
    TEST_BLOB_PROCESSING_TIMEOUT, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE,
};

use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, MAX_BATCH_EXECUTION_TIME_MILLIS};

generate_runtime!(
    name: TestRuntime,
    modules: [sequencing_data_tester: SequencingDataTester<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::config::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, Self>,
    auth_call_wrapper: |auth_data| auth_data,
);

type S = TestSpec;
type RT = TestRuntime<S>;
type TestBlueprint = RtAgnosticBlueprint<S, RT>;

fn create_genesis_params() -> (GenesisParams<GenesisConfig<S>>, TestUser<S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <RT as Runtime<S>>::GenesisConfig::from_minimal_config(genesis_config.into(), ());

    (
        GenesisParams {
            runtime: rt_genesis_config,
        },
        admin,
    )
}

async fn create_test_rollup() -> (TestRollup<TestBlueprint>, TestUser<S>) {
    let (genesis_params, admin) = create_genesis_params();
    let dir = tempdir_inside_codebase_dir();

    let rollup = new_test_rollup::<RT>(
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
        BlockProducingConfig::OnBatchSubmit {
            block_wait_timeout_ms: None,
        },
        None,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
        None,
        TEST_FINALIZATION_BLOCKS,
    )
    .await;

    (rollup, admin)
}

#[tokio::test(flavor = "multi_thread")]
async fn sequencer_sets_timestamp_before_execution() {
    let (test_rollup, admin) = create_test_rollup().await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let call = <RT as EncodeCall<SequencingDataTester<S>>>::to_decodable(
        CallMessage::AssertTimestampIsReasonable,
    );
    let before = HDTimestamp::now();
    let (published_tx, receipt) = submit_and_publish_tx(&test_rollup, &admin, call).await;
    let after = HDTimestamp::now();

    assert_eq!(
        receipt,
        TxReceiptResult::Successful,
        "the module should observe the sequencing timestamp through the transaction context"
    );

    let timestamp = decode_sequencing_data(&published_tx)
        .timestamp
        .expect("published transaction should include the sequencer timestamp");
    assert!(
        before <= timestamp && timestamp <= after,
        "published timestamp {timestamp} should have been generated during tx acceptance, between {before} and {after}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sequencer_publishes_timestamp_even_when_unused() {
    let (test_rollup, admin) = create_test_rollup().await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let call = <RT as EncodeCall<SequencingDataTester<S>>>::to_decodable(CallMessage::Noop);
    let (published_tx, _receipt) = submit_and_publish_tx(&test_rollup, &admin, call).await;

    let sequencing_data = decode_sequencing_data(&published_tx);
    assert!(
        sequencing_data.timestamp.is_some(),
        "the sequencer timestamp must be published even when execution never reads it"
    );
    assert_eq!(
        sequencing_data.unrecorded_data(),
        None,
        "a runtime without format-specific sequencing data must not publish a data payload"
    );
}

fn decode_sequencing_data(tx: &FullyBakedTx) -> SequencingData {
    let bytes = tx
        .sequencing_data
        .as_ref()
        .expect("published transaction should include sequencing data");
    SequencingData::decode(bytes).expect("published sequencing data should decode")
}

async fn submit_and_publish_tx(
    test_rollup: &TestRollup<TestBlueprint>,
    admin: &TestUser<S>,
    call: <RT as sov_modules_api::DispatchCall>::Decodable,
) -> (FullyBakedTx, TxReceiptResult) {
    let tx =
        default_test_signed_transaction::<RT, S>(&admin.private_key, &call, 0, &RT::CHAIN_HASH);
    let raw_tx = RawTx::new(to_vec(&tx).unwrap());

    let baked_tx =
        <<RT as Runtime<S>>::Auth as TransactionAuthenticator<S>>::encode_with_standard_auth(
            raw_tx.clone(),
        );
    assert!(
        baked_tx.sequencing_data.is_none(),
        "test must submit a tx without sequencing metadata"
    );

    let tx_hash = <RT as Runtime<S>>::Auth::compute_tx_hash(&baked_tx)
        .expect("submitted transaction hash should compute");

    let response = test_rollup
        .api_client()
        .send_raw_tx_to_sequencer(&raw_tx)
        .await
        .expect("sequencer should accept the transaction");
    let receipt = response
        .into_inner()
        .receipt
        .expect("sequencer confirmation should include a receipt")
        .result;
    let last_checked_height = test_rollup
        .da_service
        .get_head_block_header()
        .await
        .unwrap()
        .height;
    test_rollup.force_close_batch().await.unwrap();

    let mut published = test_rollup
        .wait_for_txs_on_da(
            &BTreeSet::from([tx_hash]),
            last_checked_height,
            Duration::from_secs(15),
        )
        .await
        .expect("published batch blobs should decode");
    (
        published
            .remove(&tx_hash)
            .expect("the submitted tx should be published"),
        receipt,
    )
}

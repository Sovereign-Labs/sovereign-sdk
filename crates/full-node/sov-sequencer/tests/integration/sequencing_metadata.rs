use borsh::to_vec;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::{EncodeCall, RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_test_modules::sequencing_data::SequencingDataTester;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::{
    default_test_signed_transaction, generate_optimistic_runtime_with_kernel, TestSpec, TestUser,
    TEST_BLOB_PROCESSING_TIMEOUT, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE,
};

use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, MAX_BATCH_EXECUTION_TIME_MILLIS};

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [sequencing_data_tester: SequencingDataTester<S>],
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

    (GenesisParams { runtime: rt_genesis_config }, admin)
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

    let call =
        <RT as EncodeCall<SequencingDataTester<S>>>::to_decodable(());
    let tx = default_test_signed_transaction::<RT, S>(
        &admin.private_key,
        &call,
        0,
        &RT::CHAIN_HASH,
    );
    let raw_tx = RawTx::new(to_vec(&tx).unwrap());

    let baked_tx =
        <<RT as Runtime<S>>::Auth as TransactionAuthenticator<S>>::encode_with_standard_auth(
            raw_tx.clone(),
        );
    assert!(
        baked_tx.sequencing_data.is_none(),
        "test must submit a tx without sequencing metadata"
    );

    test_rollup
        .api_client()
        .send_raw_tx_to_sequencer(&raw_tx)
        .await
        .expect("sequencer execution should insert sequencing metadata");
}

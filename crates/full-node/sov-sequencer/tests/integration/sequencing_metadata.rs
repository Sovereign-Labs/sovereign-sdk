use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

use borsh::to_vec;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::{
    Guard, HasCapabilities, HasSequencingData, SequencingDataView, TransactionAuthenticator,
};
use sov_modules_api::{
    Context, EncodeCall, FullyBakedTx, HDTimestamp, RawTx, Runtime, Spec, TxState,
};
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::node::da::DaService;
use sov_test_modules::sequencing_data::{CallMessage, SequencingDataTester};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::StandardProvenRollupCapabilities;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::{
    default_test_signed_transaction, generate_runtime_without_capabilities, TestSpec, TestUser,
    TEST_BLOB_PROCESSING_TIMEOUT, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE,
};

use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, MAX_BATCH_EXECUTION_TIME_MILLIS};

const INITIAL_TIMESTAMP_NANOS: u128 = 1_893_456_000_000_000_000;
static ACCESS_TIMESTAMP_IN_HANDLER: AtomicBool = AtomicBool::new(true);
static SEQUENCING_METADATA_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

struct TimestampAccessMode;

impl TimestampAccessMode {
    fn set(access_timestamp: bool) -> Self {
        ACCESS_TIMESTAMP_IN_HANDLER.store(access_timestamp, Ordering::SeqCst);
        Self
    }
}

impl Drop for TimestampAccessMode {
    fn drop(&mut self) {
        ACCESS_TIMESTAMP_IN_HANDLER.store(true, Ordering::SeqCst);
    }
}

generate_runtime_without_capabilities!(
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

impl<S: Spec> HasCapabilities<S> for TestRuntime<S> {
    type Capabilities<'a>
        = StandardProvenRollupCapabilities<'a, S>
    where
        Self: 'a;

    fn capabilities(&mut self) -> Guard<Self::Capabilities<'_>> {
        Guard::new(StandardProvenRollupCapabilities {
            bank: &mut self.bank,
            gas_payer: (),
            sequencer_registry: &mut self.sequencer_registry,
            accounts: &mut self.accounts,
            uniqueness: &mut self.uniqueness,
            chain_state: &mut self.chain_state,
            operator_incentives: &mut self.operator_incentives,
            prover_incentives: &mut self.prover_incentives,
            attester_incentives: &mut self.attester_incentives,
        })
    }
}

impl<S: Spec> HasSequencingData<S> for TestRuntime<S> {
    type SequencingData = HDTimestamp;

    fn handle_sequencing_data(
        &mut self,
        data: &SequencingDataView<'_, Self::SequencingData>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if !context.sequencer_is_preferred() {
            return Ok(());
        }

        if !ACCESS_TIMESTAMP_IN_HANDLER.load(Ordering::SeqCst) {
            return Ok(());
        }

        if let Some(timestamp) = data.get(&())? {
            self.chain_state
                .update_oracle_time_from_sequencing_data(timestamp, state)?;
        }

        Ok(())
    }

    fn create_sequencing_data(&self) -> Option<sov_modules_api::Bytes> {
        Some(
            to_vec(&timestamp_from_nanos(INITIAL_TIMESTAMP_NANOS))
                .expect("timestamp serialization should be infallible")
                .into(),
        )
    }
}

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
    let _test_lock = SEQUENCING_METADATA_TEST_LOCK.lock().await;
    let _access_mode = TimestampAccessMode::set(true);
    let (test_rollup, admin) = create_test_rollup().await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let call = <RT as EncodeCall<SequencingDataTester<S>>>::to_decodable(
        CallMessage::AssertTimestampIsReasonable,
    );
    let published_tx = submit_and_publish_tx(&test_rollup, &admin, call).await;

    let sequencing_data = published_tx
        .sequencing_data
        .expect("published transaction should include finalized sequencing metadata");
    assert_eq!(
        timestamp_bytes_to_nanos(&sequencing_data).unwrap(),
        INITIAL_TIMESTAMP_NANOS,
        "sequencer should preserve metadata accessed during execution"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sequencer_prunes_timestamp_when_not_accessed() {
    let _test_lock = SEQUENCING_METADATA_TEST_LOCK.lock().await;
    let _access_mode = TimestampAccessMode::set(false);
    let (test_rollup, admin) = create_test_rollup().await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let call = <RT as EncodeCall<SequencingDataTester<S>>>::to_decodable(CallMessage::Noop);
    let published_tx = submit_and_publish_tx(&test_rollup, &admin, call).await;

    assert!(
        published_tx.sequencing_data.is_none(),
        "sequencer should prune timestamp metadata that was not accessed during execution"
    );
}

async fn submit_and_publish_tx(
    test_rollup: &TestRollup<TestBlueprint>,
    admin: &TestUser<S>,
    call: <RT as sov_modules_api::DispatchCall>::Decodable,
) -> FullyBakedTx {
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

    test_rollup
        .api_client()
        .send_raw_tx_to_sequencer(&raw_tx)
        .await
        .expect("sequencer execution should insert sequencing metadata");
    let last_checked_height = test_rollup
        .da_service
        .get_head_block_header()
        .await
        .unwrap()
        .height;
    test_rollup.force_close_batch().await.unwrap();

    let mut published = test_rollup
        .wait_for_txs_on_da::<RT>(
            &BTreeSet::from([tx_hash]),
            last_checked_height,
            Duration::from_secs(15),
        )
        .await
        .expect("published batch blobs should decode");
    published
        .remove(&tx_hash)
        .expect("the submitted tx should be published")
}

fn timestamp_from_nanos(nanos: u128) -> HDTimestamp {
    HDTimestamp::from_str(&nanos.to_string()).expect("u128 timestamp should parse")
}

fn timestamp_bytes_to_nanos(bytes: &[u8]) -> anyhow::Result<u128> {
    let bytes = <[u8; 16]>::try_from(bytes)?;
    Ok(u128::from_le_bytes(bytes))
}

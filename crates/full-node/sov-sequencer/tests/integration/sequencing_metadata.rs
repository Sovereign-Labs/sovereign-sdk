use std::str::FromStr;
use std::time::Duration;

use borsh::{to_vec, BorshDeserialize};
use sov_blob_storage::PreferredBatchData;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::{
    Guard, HasCapabilities, SequencingDataHandler, TransactionAuthenticator,
};
use sov_modules_api::{
    BlobReaderTrait, Context, EncodeCall, HDTimestamp, RawTx, Runtime, Spec, TxState,
};
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::node::da::DaService;
use sov_test_modules::sequencing_data::{SequencingDataTester, SCRATCHPAD_TIMESTAMP_NANOS};
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

struct TestSequencingDataHandler<'a, S: Spec> {
    chain_state: &'a mut sov_test_utils::runtime::ChainState<S>,
}

impl<S: Spec> SequencingDataHandler<S> for TestSequencingDataHandler<'_, S> {
    type SequencingData = HDTimestamp;

    fn handle_sequencing_data(
        &mut self,
        data: Self::SequencingData,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if !context.sequencer_is_preferred() {
            return Ok(());
        }

        self.chain_state
            .update_oracle_time_from_sequencing_data(data, state)
    }

    fn create_sequencing_data(&self) -> Self::SequencingData {
        timestamp_from_nanos(INITIAL_TIMESTAMP_NANOS)
    }

    fn finalize_sequencing_data(
        &mut self,
        data: Self::SequencingData,
        scratchpad: Option<sov_rollup_interface::Bytes>,
    ) -> Self::SequencingData {
        scratchpad
            .as_deref()
            .and_then(|bytes| timestamp_bytes_to_nanos(bytes).ok())
            .map(timestamp_from_nanos)
            .unwrap_or(data)
    }
}

impl<S: Spec> HasCapabilities<S> for TestRuntime<S> {
    type Capabilities<'a>
        = StandardProvenRollupCapabilities<'a, S>
    where
        Self: 'a;
    type SequencingData = HDTimestamp;

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

    fn sequencing_data_handler(
        &mut self,
    ) -> impl SequencingDataHandler<S, SequencingData = Self::SequencingData> {
        TestSequencingDataHandler {
            chain_state: &mut self.chain_state,
        }
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
    let (test_rollup, admin) = create_test_rollup().await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let call = <RT as EncodeCall<SequencingDataTester<S>>>::to_decodable(());
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

    let accepted_tx = test_rollup
        .api_client()
        .send_raw_tx_to_sequencer(&raw_tx)
        .await
        .expect("sequencer execution should insert sequencing metadata");
    let accepted_tx_hash = accepted_tx.as_ref().id.to_string();
    let mut last_checked_height = test_rollup
        .da_service
        .get_head_block_header()
        .await
        .unwrap()
        .height;
    test_rollup.force_close_batch().await.unwrap();

    let published_tx = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            test_rollup.da_service.produce_block_now().await.unwrap();
            let head_height = test_rollup
                .da_service
                .get_head_block_header()
                .await
                .unwrap()
                .height;
            for height in last_checked_height + 1..=head_height {
                let mut block = test_rollup.da_service.get_block_at(height).await.unwrap();
                for blob in block.batch_blobs.iter_mut() {
                    let batch = PreferredBatchData::try_from_slice(blob.full_data())
                        .expect("preferred batch blob should decode");
                    if let Some(tx) = batch.data.iter().find_map(|tx| {
                        let hash = <RT as Runtime<S>>::Auth::compute_tx_hash(tx)
                            .expect("published transaction hash should compute");
                        (hash.to_string() == accepted_tx_hash).then(|| tx.clone())
                    }) {
                        return tx;
                    }
                }
            }
            last_checked_height = head_height;
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for the submitted tx to be published");
    let sequencing_data = published_tx
        .sequencing_data
        .expect("published transaction should include finalized sequencing metadata");
    assert_eq!(
        timestamp_bytes_to_nanos(&sequencing_data).unwrap(),
        SCRATCHPAD_TIMESTAMP_NANOS,
        "sequencer should finalize metadata using the execution scratchpad"
    );
}

fn timestamp_from_nanos(nanos: u128) -> HDTimestamp {
    HDTimestamp::from_str(&nanos.to_string()).expect("u128 timestamp should parse")
}

fn timestamp_bytes_to_nanos(bytes: &[u8]) -> anyhow::Result<u128> {
    let bytes = <[u8; 16]>::try_from(bytes)?;
    Ok(u128::from_le_bytes(bytes))
}

mod replica_tests;

use crate::preferred_end_to_end::tx_set_value;
use crate::preferred_end_to_end::{TestBlueprint, TestRuntime};
use crate::utils::get_height;
use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, MAX_BATCH_EXECUTION_TIME_MILLIS};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::types as api_types;
use sov_api_spec::types::TxInfoWithConfirmation;
use sov_api_spec::ResponseValue;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::Runtime;
use sov_modules_stf_blueprint::GenesisParams;
use sov_node_client::NodeClient;
use sov_paymaster::PaymasterConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::{
    TestSpec, TestUser, TEST_BLOB_PROCESSING_TIMEOUT, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE,
};
use sov_value_setter::ValueSetterConfig;
use std::sync::Arc;
use tokio::time::Duration;

async fn create_test_rollups(
    num_replicas: u64,
) -> (
    Option<Vec<TestRollup<TestBlueprint>>>,
    Arc<tempfile::TempDir>,
    TestUser<TestSpec>,
) {
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
            num_replicas,
            MAX_BATCH_EXECUTION_TIME_MILLIS,
            None,
            TEST_FINALIZATION_BLOCKS,
        )
        .await,
        dir,
        admin,
    )
}

#[derive(Debug, serde::Deserialize)]
struct ValueResponse {
    #[allow(unused)]
    value: u32,
}

async fn query_value(client: &NodeClient) -> u32 {
    let response = client
        .query_rest_endpoint::<ValueResponse>("/modules/value-setter/state/value")
        .await
        .unwrap();
    response.value
}

async fn send_set_value_tx(
    client: &sov_api_spec::client::Client,
    priv_key: &Ed25519PrivateKey,
    generation: u64,
    value_to_set: u64,
) -> Result<ResponseValue<TxInfoWithConfirmation>, sov_api_spec::Error<api_types::ApiError>> {
    let tx = tx_set_value(priv_key, generation, value_to_set);
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
}

async fn wait_for_height(client: &NodeClient, da_service: &StorableMockDaService, height: u64) {
    let mut current_height = get_height(client).await.unwrap();
    while current_height.get() < height {
        da_service.produce_block_now().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        current_height = get_height(client).await.unwrap();
    }
}

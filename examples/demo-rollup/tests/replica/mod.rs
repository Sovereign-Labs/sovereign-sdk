use crate::test_helpers::build_transfer_token_tx;
use crate::test_helpers::test_genesis_source;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_api_spec::types;
use sov_bank::config_gas_token_id;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_full_node_configs::sequencer::SequencerKindConfig;
use sov_mock_da::storable::rpc::start_server;
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaConfig};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::CryptoSpec;
use sov_modules_api::OperatingMode;
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_sequencer::preferred::NodeRole;
use sov_test_utils::postgres::CreatePostgresError;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::PostgresData;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::error::Elapsed;
use tokio::time::Duration;

mod replica_gets_txs_from_master;
mod start_stop;

type S = <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
const TEST_SEQ_DA_ADDRESS: MockAddress = MockAddress::new([0; 32]);
const AMOUNT: u128 = 100;

fn random_address() -> <S as Spec>::Address {
    let pk = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    pk.pub_key().credential_id().into()
}

// Actually creates DA service that produces block on batch submitted.
async fn create_da_service_manual() -> (StorableMockDaService, SocketAddr) {
    let da_service = StorableMockDaService::new_in_memory(TEST_SEQ_DA_ADDRESS, 0).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    (da_service, addr)
}

async fn create_da_service_periodic() -> (StorableMockDaService, watch::Sender<()>, SocketAddr) {
    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(());
    let mut da_config = MockDaConfig::instant_with_sender(TEST_SEQ_DA_ADDRESS);
    da_config.block_producing = BlockProducingConfig::Periodic {
        block_time_ms: TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS * 2,
    };

    let da_service = StorableMockDaService::from_config(da_config, shutdown_receiver).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    (da_service, shutdown_sender, addr)
}

async fn start_rollup(
    addr: SocketAddr,
    postgres: Option<(Arc<PostgresData>, String, NodeRole)>,
) -> TestRollup<ExternalMockDemoRollup<Native>> {
    let genesis = test_genesis_source(OperatingMode::Operator);
    RollupBuilder::new_with_external_da(
        genesis,
        MockDaClientConfig {
            url: format!("http://{addr}"),
        },
        postgres,
    )
    .await
    .set_config(|c| match &mut c.sequencer_config {
        SequencerKindConfig::Standard(_) => {}
        SequencerKindConfig::Preferred(p) => {
            p.num_cache_warmup_workers = 0;
        }
    })
    .start_test_rollup()
    .await
    .unwrap()
}

async fn send_transfers(
    start_nonce: u64,
    count: u64,
    key_and_address: PrivateKeyAndAddress<S>,
    receiver: <S as Spec>::Address,
    test_rollup: &TestRollup<ExternalMockDemoRollup<Native>>,
) {
    for n in 0..count {
        println!("Sending tx {n}");
        let tx = build_transfer_token_tx::<S>(
            &key_and_address.private_key,
            config_gas_token_id(),
            receiver,
            AMOUNT,
            start_nonce + n,
        );
        let mut retry_duration = Duration::from_millis(100);
        // Send the tx with retries, up to 7 attempts (about 30 seconds)
        for attempt in 1..=7 {
            match test_rollup.send_tx_to_sequencer(&tx).await {
                Ok(_) => break,
                Err(e) => {
                    if e.to_string().contains("The node fell out of sync") {
                        if attempt == 7 {
                            panic!("Failed to send tx after 7 attempts (about 30 seconds). This probably means that the test is too heavy.: {e}");
                        }
                        tokio::time::sleep(retry_duration).await;
                        retry_duration *= 2;
                    } else {
                        panic!("Unexpected error while sending tx: {e}");
                    }
                }
            }
        }
        // Wait between each successful send, to avoid overwhelming the sequencer
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_all_events_with_timeout(
    timeout: Duration,
    nb_of_events: u64,
    subscription: &mut BoxStream<'static, anyhow::Result<types::LedgerEvent>>,
) -> Result<(), Elapsed> {
    tokio::time::timeout(timeout, wait_for_all_events(nb_of_events, subscription)).await
}

async fn wait_for_all_events(
    nb_of_events: u64,
    subscription: &mut BoxStream<'static, anyhow::Result<types::LedgerEvent>>,
) {
    println!("Waiting for {nb_of_events} events");
    for i in 0..nb_of_events {
        println!("Waiting for event {i}");
        let _ = subscription.next().await.unwrap();
    }
}

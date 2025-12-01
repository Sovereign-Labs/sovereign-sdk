use crate::test_helpers::build_transfer_token_tx;
use crate::test_helpers::test_genesis_source;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_api_spec::types;
use sov_bank::config_gas_token_id;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_mock_da::storable::rpc::start_server;
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{MockAddress, MockDaConfig};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::CryptoSpec;
use sov_modules_api::OperatingMode;
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_test_utils::postgres::CreatePostgresError;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::NodeIdAndTimeDelta;
use sov_test_utils::test_rollup::PostgresData;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::TestRollup;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::watch;
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
    da_config.block_producing = sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

    let da_service = StorableMockDaService::from_config(da_config, shutdown_receiver).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    (da_service, shutdown_sender, addr)
}

async fn start_rollup(
    is_replica: bool,
    addr: SocketAddr,
    postgres: Option<(Arc<PostgresData>, NodeIdAndTimeDelta)>,
) -> TestRollup<ExternalMockDemoRollup<Native>> {
    let genesis = test_genesis_source(OperatingMode::Operator);
    RollupBuilder::new_with_external_da(
        is_replica,
        genesis,
        MockDaClientConfig {
            url: format!("http://{addr}"),
        },
        postgres,
    )
    .await
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
        let tx = build_transfer_token_tx::<S>(
            &key_and_address.private_key,
            config_gas_token_id(),
            receiver,
            AMOUNT,
            start_nonce + n,
        );
        test_rollup.send_tx_to_sequencer(&tx).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_all_events_with_timeout(
    timeout: Duration,
    nb_of_events: u64,
    subscription: &mut BoxStream<'static, anyhow::Result<types::LedgerEvent>>,
) {
    tokio::time::timeout(timeout, wait_for_all_events(nb_of_events, subscription))
        .await
        .unwrap();
}

async fn wait_for_all_events(
    nb_of_events: u64,
    subscription: &mut BoxStream<'static, anyhow::Result<types::LedgerEvent>>,
) {
    for _ in 0..nb_of_events {
        let _ = subscription.next().await.unwrap();
    }
}

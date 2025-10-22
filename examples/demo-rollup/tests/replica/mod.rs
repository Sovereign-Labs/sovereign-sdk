use crate::test_helpers::build_transfer_token_tx;
use crate::test_helpers::test_genesis_source;
use sov_bank::config_gas_token_id;
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
use sov_test_utils::test_rollup::PostgresData;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::TestRollup;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::Duration;

type S = <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
const TEST_SEQ_DA_ADDRESS: MockAddress = MockAddress::new([0; 32]);

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

async fn wait_for_height(
    test_rollup: &TestRollup<ExternalMockDemoRollup<Native>>,
    da_service: &StorableMockDaService,
    height: u64,
) {
    loop {
        let h = test_rollup.height().await;
        da_service.produce_block_now().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        if h.get() >= height {
            break;
        }
    }
}

async fn start_rollup(
    is_replica: bool,
    addr: SocketAddr,
    postgres: Option<Arc<PostgresData>>,
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

#[tokio::test(flavor = "multi_thread")]
async fn test_replica_receives_txs_from_da() {
    let (_, shutdown_sender, addr) = create_da_service_periodic().await;
    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    let test_rollup = start_rollup(false, addr, None).await;
    let replica_test_rollup = start_rollup(true, addr, None).await;

    let token_id = config_gas_token_id();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let receiver_addr = random_address();

    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        100,
        0,
    );

    let height_before_tx = test_rollup.height().await;

    test_rollup.send_tx_to_sequencer(&tx).await.unwrap();

    test_rollup
        .wait_for_height(height_before_tx.get() + 5)
        .await;

    let receiver_balance = replica_test_rollup
        .client
        .get_balance::<S>(&receiver_addr, &token_id, None)
        .await
        .unwrap();

    assert_eq!(receiver_balance.0, 100);
    let _ = test_rollup.shutdown().await;
    let _ = shutdown_sender.send(());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_replica_receives_txs_from_postgres() {
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let (da_service, addr) = create_da_service_manual().await;
    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    let test_rollup = start_rollup(false, addr, postgres.clone()).await;
    let replica_test_rollup = start_rollup(true, addr, postgres).await;

    let token_id = config_gas_token_id();

    da_service.produce_n_blocks_now(20).await.unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let receiver_addr = random_address();

    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        100,
        0,
    );

    let height_before_tx = test_rollup.height().await;
    test_rollup.send_tx_to_sequencer(&tx).await.unwrap();

    wait_for_height(&test_rollup, &da_service, height_before_tx.get() + 5).await;

    let receiver_balance = replica_test_rollup
        .client
        .get_balance::<S>(&receiver_addr, &token_id, None)
        .await
        .unwrap();

    assert_eq!(receiver_balance.0, 100);
    let _ = test_rollup.shutdown().await;
}

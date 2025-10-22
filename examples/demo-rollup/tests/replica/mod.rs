use crate::test_helpers::build_transfer_token_tx;
use crate::test_helpers::test_genesis_source;
use sov_api_spec::types::TxStatus;
use sov_bank::config_gas_token_id;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_mock_da::storable::rpc::start_server;
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::MockAddress;
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

type S = <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;

fn random_address() -> <S as Spec>::Address {
    let pk = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    pk.pub_key().credential_id().into()
}

async fn create_da_service_manual() -> (StorableMockDaService, SocketAddr) {
    let da_service = StorableMockDaService::new_in_memory_manual(MockAddress::new([0; 32])).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    (da_service, addr)
}

async fn create_da_service() -> (StorableMockDaService, watch::Sender<()>, SocketAddr) {
    let (da_service, shutdown_sender) =
        StorableMockDaService::new_in_memory_periodic(300, MockAddress::new([0; 32])).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    (da_service, shutdown_sender, addr)
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
    let (_, shutdown_sender, addr) = create_da_service().await;
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
    sov_test_utils::logging::initialize_or_change_logging_with_filter("warn");
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
    let tx_result = test_rollup.send_tx_to_sequencer(&tx).await.unwrap();
    assert_eq!(tx_result.status, TxStatus::Processed);
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
}

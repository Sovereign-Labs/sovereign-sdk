#![allow(dead_code)]
use crate::test_helpers::build_transfer_token_tx;
use crate::test_helpers::test_genesis_source;
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
use sov_test_utils::test_rollup::PostgresData;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::TestRollup;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::watch;

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

const AMOUNT_TO_SEND: u128 = 100;

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
            AMOUNT_TO_SEND,
            start_nonce + n,
        );
        test_rollup.send_tx_to_sequencer(&tx).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

async fn produce_blocks(nb_of_blocks: usize, da_service: &StorableMockDaService) {
    for _ in 0..nb_of_blocks {
        da_service.produce_block_now().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(
            sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS,
        ))
        .await;
    }
}

struct SetupData {
    postgres: Option<Arc<PostgresData>>,
    da_service: StorableMockDaService,
    addr: SocketAddr,
    key_and_address: PrivateKeyAndAddress<S>,
}

async fn setup() -> Option<SetupData> {
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return None,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let (da_service, addr) = create_da_service_manual().await;
    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    Some(SetupData {
        postgres,
        da_service,
        addr,
        key_and_address,
    })
}

async fn setup_periodic() -> Option<(SetupData, watch::Sender<()>)> {
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return None,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let (da_service, da_shutdown_sender, addr) = create_da_service_periodic().await;
    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    Some((
        SetupData {
            postgres,
            da_service,
            addr,
            key_and_address,
        },
        da_shutdown_sender,
    ))
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

    let height_before_tx = test_rollup.height().await;
    send_transfers(0, 1, key_and_address, receiver_addr, &test_rollup).await;

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
async fn test_replica_receives_txs_from_postgres_x() {
    let Some(SetupData {
        postgres,
        da_service,
        addr,
        key_and_address,
    }) = setup().await
    else {
        return;
    };

    let replica_test_rollup = start_rollup(true, addr, postgres.clone()).await;
    println!("LOL0");
    let test_rollup = start_rollup(false, addr, postgres).await;

    println!("LOL1");
    produce_blocks(20, &da_service).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    println!("LOL2");
    let receiver_addr = random_address();

    println!("LOL3");
    send_transfers(0, 1, key_and_address, receiver_addr, &test_rollup).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let receiver_balance = replica_test_rollup
        .client
        .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
        .await
        .unwrap();

    assert_eq!(receiver_balance.0, AMOUNT_TO_SEND);
    let _ = test_rollup.shutdown().await;
    let _ = replica_test_rollup.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_replica_receives_txs_from_postgres_replica_start_after_master() {
    let Some(SetupData {
        postgres,
        da_service,
        addr,
        key_and_address,
    }) = setup().await
    else {
        return;
    };

    let test_rollup = start_rollup(false, addr, postgres.clone()).await;
    produce_blocks(20, &da_service).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    let replica_test_rollup = start_rollup(true, addr, postgres).await;
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    let receiver_addr = random_address();

    send_transfers(0, 1, key_and_address, receiver_addr, &test_rollup).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let receiver_balance = replica_test_rollup
        .client
        .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
        .await
        .unwrap();

    assert_eq!(receiver_balance.0, AMOUNT_TO_SEND);
    let _ = test_rollup.shutdown().await;
    let _ = replica_test_rollup.shutdown().await;
}

/*
#[tokio::test(flavor = "multi_thread")]
async fn test_replica_start_stop() {
    let Some(SetupData {
        postgres,
        da_service,
        addr,
        key_and_address,
    }) = setup().await
    else {
        return;
    };

    let replica_test_rollup = start_rollup(true, addr, postgres.clone()).await;
    let test_rollup = start_rollup(false, addr, postgres).await;
    produce_blocks(20, &da_service).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let receiver_addr = random_address();

    let nb_of_txs = 1000;
    println!("X1");
    {
        send_transfers(
            0,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, (nb_of_txs as u128) * AMOUNT_TO_SEND);
    }

    println!("X2");
    // Restart the rollup
    let test_rollup = {
        let builder = test_rollup.shutdown().await.unwrap();
        produce_blocks(5, &da_service).await;
        let test_rollup = builder.start_test_rollup().await.unwrap();

        //produce_blocks(50, &da_service).await;
        //test_rollup.wait_for_sequencer_ready().await.unwrap();
        while !test_rollup.is_sequencer_ready().await {
            println!("produce blocks");
            produce_blocks(2, &da_service).await;
        }

        test_rollup
    };

    println!("X3");

    {
        send_transfers(
            nb_of_txs as u64,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, 2 * (nb_of_txs as u128) * AMOUNT_TO_SEND);
        println!("receiver balance: {}", receiver_balance.0);
    }

    produce_blocks(5, &da_service).await;
    println!("X4");
    // Restart replica
    let replica_test_rollup = {
        let builder = replica_test_rollup.shutdown().await.unwrap();
        produce_blocks(5, &da_service).await;
        let replica_test_rollup = builder.start_test_rollup().await.unwrap();
        produce_blocks(60, &da_service).await;
        replica_test_rollup
    };

    println!("X5");
    {
        send_transfers(
            2 * nb_of_txs as u64,
            nb_of_txs,
            key_and_address,
            receiver_addr,
            &test_rollup,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, 3 * (nb_of_txs as u128) * AMOUNT_TO_SEND);
        println!("receiver balance: {}", receiver_balance.0);
    }

    let _ = test_rollup.shutdown().await;
    let _ = replica_test_rollup.shutdown().await;
}
*/

#[tokio::test(flavor = "multi_thread")]
async fn test_replica_start_stop() {
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let (da_service, da_shutdown, addr) = create_da_service_periodic().await;
    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    let replica_test_rollup = start_rollup(true, addr, postgres.clone()).await;
    let test_rollup = start_rollup(false, addr, postgres).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let receiver_addr = random_address();

    let nb_of_txs = 1500;
    println!("X1");
    {
        send_transfers(
            0,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, (nb_of_txs as u128) * AMOUNT_TO_SEND);
    }

    println!("X2");
    // Restart the rollup
    let test_rollup = {
        let builder = test_rollup.shutdown().await.unwrap();
        let test_rollup = builder.start_test_rollup().await.unwrap();
        test_rollup.wait_for_sequencer_ready().await.unwrap();
        test_rollup
    };

    println!("X3");

    {
        send_transfers(
            nb_of_txs as u64,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, 2 * (nb_of_txs as u128) * AMOUNT_TO_SEND);
        println!("receiver balance: {}", receiver_balance.0);
    }

    println!("X4");
    // Restart replica
    let replica_test_rollup = {
        let builder = replica_test_rollup.shutdown().await.unwrap();
        let replica_test_rollup = builder.start_test_rollup().await.unwrap();
        replica_test_rollup
    };

    println!("X5");
    {
        send_transfers(
            2 * nb_of_txs as u64,
            nb_of_txs,
            key_and_address,
            receiver_addr,
            &test_rollup,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, 3 * (nb_of_txs as u128) * AMOUNT_TO_SEND);
        println!("receiver balance: {}", receiver_balance.0);
    }

    let _ = test_rollup.shutdown().await;
    let _ = replica_test_rollup.shutdown().await;
}

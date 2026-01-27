use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn test_replica_receives_txs_from_da() {
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");
    let (_, da_shutdown, addr) = create_da_service_periodic().await;

    let replica = postgres
        .clone()
        .map(|pg| (pg, "replica".into(), ConfiguredNodeRole::ReplicaNoLeaderSync));
    let replica_test_rollup = start_rollup(addr, replica).await;

    let primary = postgres
        .clone()
        .map(|pg| (pg, "primary".into(), ConfiguredNodeRole::Leader));
    let test_rollup = start_rollup(addr, primary).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let token_id = config_gas_token_id();
    let receiver_addr = random_address();

    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        AMOUNT,
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

    assert_eq!(receiver_balance.0, AMOUNT);

    let _ = replica_test_rollup.shutdown().await;
    let _ = test_rollup.shutdown().await;
    let _ = da_shutdown.send(());
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

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");
    let (da_service, addr) = create_da_service_manual().await;

    let replica = postgres
        .clone()
        .map(|pg| (pg, "replica".into(), ConfiguredNodeRole::Replica));
    let replica_test_rollup = start_rollup(addr, replica).await;

    let primary = postgres
        .clone()
        .map(|pg| (pg, "primary".into(), ConfiguredNodeRole::Leader));
    let test_rollup = start_rollup(addr, primary).await;

    for _ in 0..20 {
        da_service.produce_block_now().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(
            sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS,
        ))
        .await;
    }

    test_rollup.wait_for_sequencer_ready().await.unwrap();
    replica_test_rollup
        .wait_for_sequencer_ready()
        .await
        .unwrap();

    let mut event_subscription = replica_test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    let token_id = config_gas_token_id();
    let receiver_addr = random_address();

    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        AMOUNT,
        0,
    );

    test_rollup.send_tx_to_sequencer(&tx).await.unwrap();
    wait_for_all_events_with_timeout(Duration::from_millis(100), 1, &mut event_subscription).await;

    let receiver_balance = replica_test_rollup
        .client
        .get_balance::<S>(&receiver_addr, &token_id, None)
        .await
        .unwrap();

    assert_eq!(receiver_balance.0, AMOUNT);
    let _ = replica_test_rollup.shutdown().await;
    let _ = test_rollup.shutdown().await;
}

use super::*;

use tokio::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn test_replica_start_stop() {
    sov_test_utils::logging::initialize_or_change_logging_with_filter("info,tower=off");
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let (da_service, da_shutdown, addr) = create_da_service_periodic().await;
    // Ideal lag and stuff
    da_service.wait_for_height(10).await.unwrap();

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    let replica = postgres
        .clone()
        .map(|pg| (pg, "replica".into(), ConfiguredNodeRole::Replica));
    let replica_test_rollup = start_rollup(addr, replica).await;

    let primary = postgres
        .clone()
        .map(|pg| (pg, "primary".into(), ConfiguredNodeRole::Leader));
    let test_rollup = start_rollup(addr, primary).await;

    replica_test_rollup
        .wait_for_sequencer_ready()
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let receiver_addr = random_address();

    let nb_of_txs = 50;

    tracing::info!("===== 1 BEGIN");
    {
        let mut event_subscription = replica_test_rollup
            .api_client()
            .subscribe_to_events_with_filter("Bank/*")
            .await
            .unwrap();

        send_transfers(
            0,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;

        wait_for_all_events_with_timeout(
            Duration::from_millis(1000),
            nb_of_txs,
            &mut event_subscription,
        )
        .await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, (nb_of_txs as u128) * AMOUNT);
    }

    tracing::info!("=== IN TEST: RESTART MASTER 1");
    // Restart master.
    let test_rollup = {
        let builder = test_rollup.shutdown().await.unwrap();
        let test_rollup = builder.start_test_rollup().await.unwrap();
        test_rollup.wait_for_sequencer_ready().await.unwrap();
        test_rollup
    };

    if let Some(pg_data) = &postgres {
        sov_test_utils::docker::print_logs_from_container("postgres", &pg_data.postgres).await;
    }

    tracing::info!("=== IN TEST: WAIT FOR EVENTS 0");
    {
        let mut event_subscription = replica_test_rollup
            .api_client()
            .subscribe_to_events_with_filter("Bank/*")
            .await
            .unwrap();

        send_transfers(
            nb_of_txs,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;

        wait_for_all_events_with_timeout(
            Duration::from_millis(300),
            nb_of_txs,
            &mut event_subscription,
        )
        .await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        // ERROR IS HERE
        assert_eq!(receiver_balance.0, 2 * (nb_of_txs as u128) * AMOUNT);
    }

    tracing::info!("=== IN TEST: RESTART REPLICA");
    if let Some(pg_data) = &postgres {
        sov_test_utils::docker::print_logs_from_container("postgres", &pg_data.postgres).await;
    }

    // Restart replica
    let replica_test_rollup = {
        let builder = replica_test_rollup.shutdown().await.unwrap();
        let replica_test_rollup = builder.start_test_rollup().await.unwrap();
        replica_test_rollup
            .wait_for_sequencer_ready()
            .await
            .unwrap();
        replica_test_rollup
    };

    tracing::info!("=== IN TEST: READING EVENTS 1");
    if let Some(pg_data) = &postgres {
        sov_test_utils::docker::print_logs_from_container("postgres", &pg_data.postgres).await;
    }

    {
        let mut event_subscription = replica_test_rollup
            .api_client()
            .subscribe_to_events_with_filter("Bank/*")
            .await
            .unwrap();

        send_transfers(
            2 * nb_of_txs,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;

        wait_for_all_events_with_timeout(
            Duration::from_millis(1000),
            nb_of_txs,
            &mut event_subscription,
        )
        .await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, 3 * (nb_of_txs as u128) * AMOUNT);
    }

    tracing::info!("=== IN TEST: RESTART REPLICA AND WAIT");
    if let Some(pg_data) = &postgres {
        sov_test_utils::docker::print_logs_from_container("postgres", &pg_data.postgres).await;
    }

    // Restart replica and wait
    let replica_test_rollup = {
        let builder = replica_test_rollup.shutdown().await.unwrap();

        let height_before_tx = test_rollup.height().await;
        test_rollup
            .wait_for_height(height_before_tx.get() + 25)
            .await;

        let replica_test_rollup = builder.start_test_rollup().await.unwrap();
        replica_test_rollup
            .wait_for_sequencer_ready()
            .await
            .unwrap();
        replica_test_rollup
    };

    tracing::info!("=== IN TEST: READ EVENTS AGAIN");
    if let Some(pg_data) = &postgres {
        sov_test_utils::docker::print_logs_from_container("postgres", &pg_data.postgres).await;
    }
    {
        let mut event_subscription = replica_test_rollup
            .api_client()
            .subscribe_to_events_with_filter("Bank/*")
            .await
            .unwrap();

        send_transfers(
            3 * nb_of_txs,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;

        wait_for_all_events_with_timeout(
            Duration::from_millis(1000),
            nb_of_txs,
            &mut event_subscription,
        )
        .await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        assert_eq!(receiver_balance.0, 4 * (nb_of_txs as u128) * AMOUNT);
    }

    if let Some(pg_data) = &postgres {
        sov_test_utils::docker::print_logs_from_container("postgres", &pg_data.postgres).await;
    }

    let _ = replica_test_rollup.shutdown().await;
    let _ = test_rollup.shutdown().await;
    let _ = da_shutdown.send(());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_replica_start_stop_many_times() {
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let (_da_service, da_shutdown, addr) = create_da_service_periodic().await;
    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");
    let receiver_addr = random_address();

    let primary = postgres
        .clone()
        .map(|pg| (pg, "primary".into(), ConfiguredNodeRole::Leader));
    let test_rollup = start_rollup(addr, primary).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let replica = postgres
        .clone()
        .map(|pg| (pg, "replica".into(), ConfiguredNodeRole::Replica));
    let mut replica_test_rollup = start_rollup(addr, replica).await;

    let nb_of_txs = 50;
    for i in 0..3 {
        let builder = replica_test_rollup.shutdown().await.unwrap();

        let height_before_tx = test_rollup.height().await;
        test_rollup
            .wait_for_height(height_before_tx.get() + 25)
            .await;

        replica_test_rollup = builder.start_test_rollup().await.unwrap();
        replica_test_rollup
            .wait_for_sequencer_ready()
            .await
            .unwrap();

        let mut event_subscription = replica_test_rollup
            .api_client()
            .subscribe_to_events_with_filter("Bank/*")
            .await
            .unwrap();

        send_transfers(
            i * nb_of_txs,
            nb_of_txs,
            key_and_address.clone(),
            receiver_addr,
            &test_rollup,
        )
        .await;

        wait_for_all_events_with_timeout(
            Duration::from_millis(1000),
            nb_of_txs,
            &mut event_subscription,
        )
        .await;

        let receiver_balance = replica_test_rollup
            .client
            .get_balance::<S>(&receiver_addr, &config_gas_token_id(), None)
            .await
            .unwrap();

        let expected_balance = AMOUNT * ((nb_of_txs * (i + 1)) as u128);
        assert_eq!(receiver_balance.0, expected_balance);
    }

    let _ = replica_test_rollup.shutdown().await;
    let _ = test_rollup.shutdown().await;
    let _ = da_shutdown.send(());
}

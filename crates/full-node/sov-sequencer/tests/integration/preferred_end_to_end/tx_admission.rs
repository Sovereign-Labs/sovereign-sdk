use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_priority() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    let mut event_subscription = test_rollup
        .api_client()
        .subscribe_to_events()
        .await
        .unwrap();

    // Send a transaction which will block the sequencer for a while. This gives us time to send other txs with different priorities and ensure
    // that the priority tiebreaker is working.
    let client = test_rollup.api_client().clone();
    let tx = tx_set_value_and_sleep(&admin.private_key, 0, 1000, 5000);
    tokio::spawn(async move {
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
    });
    // Sleep for a little while to ensure the tx has been received and is currently blocking the sequencer.
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Send transactions in from 1 to 10 concurrently.
    for i in 1..10 {
        let tx = tx_set_value(&admin.private_key, 0, i);
        let client = test_rollup.api_client().clone();
        tokio::spawn(async move {
            client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await
                .unwrap();
        });
    }

    fn assert_event_value(event: &sov_api_spec::types::LedgerEvent, expected_value: u32) {
        assert_eq!(
            event.value.get("new_value").unwrap(),
            &serde_json::Value::Number(Number::from(expected_value)),
            "Event value is not {expected_value}. {event:?}"
        );
    }

    // Check that our initial tx arrived first and was processed - otherwise the priority numbers won't have had any effect.
    let initial_event = event_subscription.next().await.unwrap().unwrap();
    assert_event_value(&initial_event, 1000);

    // No matter what order txs arrived in, events should always be received in reverse order because higher numbers get higher priority.
    for i in (1..10).rev() {
        let event = event_subscription.next().await.unwrap().unwrap();
        assert_event_value(&event, i);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn txs_below_min_fee_are_rejected() {
    let (test_rollup, admin) = create_test_rollup_with(|b| b.minimum_profit_per_tx(1)).await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 7);
    let error = client
        .send_raw_tx_to_sequencer(&tx)
        .await
        .expect_err("Tx must have been rejected for insufficient fee");
    let err_message = error.to_string();
    assert!(
        err_message.contains("This transaction did not pay a sufficient net fee."),
        "Full error message does not contain expect part: {err_message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_txs_are_rejected() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 7);
    let mut tx_mal = tx.clone();
    tx_mal.data.push(1);
    let _ok = client
        .send_raw_tx_to_sequencer(&tx)
        .await
        .expect("Tx must have been accepted");

    let error = client
        .send_raw_tx_to_sequencer(&tx_mal)
        .await
        .expect_err("Tx must have been rejected for trailing bytes");
    let err_message = error.to_string();
    assert!(
        err_message.contains("1 trailing bytes after transaction deserialization"),
        "Full error message does not contain expect part: {err_message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tx_ws_submission() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let (mut writer, mut reader) = client.connect_txs_ws().await.unwrap();

    for i in 0..3 {
        let tx = tx_set_value(&admin.private_key, i, i);
        writer.send(&tx, i.to_string()).await.unwrap();
    }
    let mut received_ids = [false, false, false];
    for _ in 0..3 {
        let msg = reader.next().await.unwrap().unwrap();
        let id: usize = msg.id.parse().unwrap();
        assert!(!received_ids[id]);
        received_ids[id] = true;
        assert_eq!(msg.contents.events.len(), 1);
        for event in msg.contents.events {
            assert!(
                event.key.starts_with("ValueSetter/NewValue"),
                "Unexpected event key: {}",
                event.key
            );
            assert_eq!(
                event.value.get("new_value").unwrap().as_u64().unwrap(),
                id as u64,
                "Unexpected event value: {:?}",
                event.value
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_out_of_gas_for_pre_checks() {
    let mut genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let max_exec_gas_per_tx = GasUnit::from(config_value!("MAX_SEQUENCER_EXEC_GAS_PER_TX"));

    // We want to set the initial gas limit to be 3/2 of the max exec gas per tx.
    let gas_limit = max_exec_gas_per_tx;
    let gas_limit = gas_limit.checked_scalar_product(3).unwrap();
    let gas_limit = gas_limit.scalar_division(2);
    let gas_limit_array = gas_limit.as_ref();
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_BLOCK_GAS_LIMIT",
        format!("{gas_limit_array:?}"),
    );
    let price_array = config_value!("INITIAL_BASE_FEE_PER_GAS");
    let gas_price = GasPrice::<2>::from([
        Amount::from(price_array[0] as u64),
        Amount::from(price_array[1] as u64),
    ]);

    let max_amount_limit = gas_limit.value(gas_price);

    // Set very high initial balance for the admin.
    genesis_config.additional_accounts_mut()[0].available_gas_balance =
        max_amount_limit.checked_mul(Amount::new(10)).unwrap();

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
            (),
        );
    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = tempdir_inside_codebase_dir();

    let test_rollup = SeqTestRollup::<TestRuntime<TestSpec>>::new(
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
    )
    .dir(dir.clone())
    .start()
    .await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    test_rollup.pause_preferred_batches().await;

    // Produce the first transaction that nearly exhausts the gas slot limit.
    {
        let gas_to_charge = gas_limit.checked_sub(max_exec_gas_per_tx).unwrap();

        let tx = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            0,
            7,
            Some(gas_to_charge),
            max_amount_limit,
        );
        client.send_raw_tx_to_sequencer(&tx).await.unwrap();

        query_set_value(&test_rollup, None, Some(7)).await.unwrap();
    }

    // The second transaction will be rejected because of the slot gas limit.
    {
        let tx = tx_set_value(&admin.private_key, 1, 8);

        let resp = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap_err();

        let error_str = resp.to_string();
        assert!(error_str.contains("More transactions were submitted that the sequencer is allowed to put into a single batch."));
    }
    test_rollup.resume_preferred_batches().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_many_invalid_txs() {
    let txs = 1000u64;
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let generation = config_value!("PAST_TRANSACTION_GENERATIONS") + txs;
    let tx = tx_set_value(&admin.private_key, generation, 1_000_000);

    client
        .send_raw_tx_to_sequencer_with_retry(&tx)
        .await
        .unwrap();

    let mut handles = Vec::with_capacity(txs as usize);
    for i in 0..txs {
        let client = client.clone();
        let da_service = test_rollup.da_service.clone();
        // Generation is always below, so each tx is going to fail
        let tx = tx_set_value(&admin.private_key, 0, i);
        handles.push(tokio::spawn(async move {
            let backoff = backon::ExponentialBuilder::default()
                .with_factor(1.5)
                .with_min_delay(Duration::from_millis(200))
                .with_max_times(10)
                .with_max_delay(Duration::from_secs(5));

            let client_for_retry = client.clone();
            let tx_for_retry = tx.clone();
            let fut = move || {
                let client = client_for_retry.clone();
                let tx = tx_for_retry.clone();
                async move { client.send_raw_tx_to_sequencer(&tx).await }
            };

            let res = fut
                .retry(backoff)
                .when(|err| {
                    // Only retry on communication errors or HTTP 5XX
                    match err {
                        sov_api_spec::Error::CommunicationError(_) => true,
                        sov_api_spec::Error::ErrorResponse(response_value) => {
                            // Check if it's a 5XX error
                            let status = response_value.status().as_u16();
                            (500..600).contains(&status)
                        }
                        sov_api_spec::Error::InvalidResponsePayload(bytes, _) => {
                            // Check if response contains 5XX status
                            std::str::from_utf8(bytes.as_ref())
                                .ok()
                                .map(|s| {
                                    // Try to parse status from the response
                                    s.contains("\"status\":5")
                                })
                                .unwrap_or(false)
                        }
                        _ => false,
                    }
                })
                .notify(|err, dur| {
                    tracing::warn!(
                        "Task {} failed to submit transaction, retrying... Error: {:?}, Duration: {:?}",
                        i, err, dur
                    );
                })
                .await;
            // Producing block to add more work for the sequencer
            if i % 5 == 0 {
                da_service.produce_block_now().await.unwrap();
            }
            assert!(res.is_err(), "Request has been accepted, when it shouldn't");
        }));
    }

    test_rollup
        .da_service
        .produce_n_blocks_now(txs as usize / 2)
        .await
        .unwrap();

    let results = future::join_all(handles).await;
    for res in results {
        res.expect("Background sender task has panicked");
    }

    let _ = tokio::time::timeout(TEST_NORMAL_SHUTDOWN_TIMEOUT, test_rollup.shutdown())
        .await
        .expect("Rollup shutdown properly");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_are_returned_in_tx_response() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 7);
    let response = client.send_raw_tx_to_sequencer(&tx).await.unwrap();

    assert_eq!(response.events.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn delayed_tx_is_processed_after_delay() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();

    let delayed_tx = tx_delayed_call(&admin.private_key, 0);
    let regular_tx = tx_set_value(&admin.private_key, 0, 7);

    let binding = api_types::AcceptTxBody {
        body: BASE64_STANDARD.encode(&delayed_tx),
    };
    let delayed_tx_future = client.accept_tx(&binding);

    // Sleep to ensure that the tx would surely be processed if it were not delayed.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let binding = api_types::AcceptTxBody {
        body: BASE64_STANDARD.encode(&regular_tx),
    };
    let regular_tx_future = client.accept_tx(&binding);

    tokio::select! {
        _ = regular_tx_future => {},
        _ = delayed_tx_future => {
            panic!("Delayed tx was processed before the regular tx");
        },
    };
}

// Checks that transactions that are not sequencer safe are rejected
// when the sender address is not configured as an admin in the sequencer config.
#[tokio::test(flavor = "multi_thread")]
async fn not_sequencer_safe_txs_are_restricted() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup
        .da_service
        .produce_n_blocks_now(10)
        .await
        .unwrap();

    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let tx = generate_paymaster_tx::<TestRuntime<TestSpec>>(admin.private_key.clone());
    {
        if let Err(e) = test_rollup
            .client
            .client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
        {
            assert!(
                e.to_string().contains("Only designated admins are allowed"),
                "Unexpected error: {e}"
            );
        } else {
            panic!("Sequencer accepted admin tx from non-admin sender");
        }
    }
}

// Regression test: a tx rejected by the sequencer's admin pre-flight check must produce exactly
// one result from the executor task. Otherwise the stale extra result is handed to the next tx,
// shifting every subsequent result by one and eventually tripping the tx hash assertion.
#[tokio::test(flavor = "multi_thread")]
async fn txs_after_not_sequencer_safe_tx_get_their_own_results() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();

    let rejected_tx = generate_paymaster_tx::<TestRuntime<TestSpec>>(admin.private_key.clone());
    let error = client
        .send_raw_tx_to_sequencer(&rejected_tx)
        .await
        .expect_err("Sequencer accepted admin tx from non-admin sender");
    assert!(
        error
            .to_string()
            .contains("Only designated admins are allowed"),
        "Unexpected error: {error}"
    );

    // With a duplicated result, this tx receives the rejected tx's (unsuccessful) result.
    let first_tx = tx_set_value(&admin.private_key, 0, 1000);
    client
        .send_raw_tx_to_sequencer(&first_tx)
        .await
        .expect("First valid tx after the rejected one should be accepted");

    // With a duplicated result, this tx receives the first tx's (successful) result, which trips
    // the sequencer's tx hash assertion and crashes the rollup.
    let second_tx = tx_set_value(&admin.private_key, 1, 1001);
    client
        .send_raw_tx_to_sequencer(&second_tx)
        .await
        .expect("Second valid tx after the rejected one should be accepted");

    test_rollup
        .shutdown()
        .await
        .expect("Rollup should shut down cleanly, without any crashed background tasks");
}

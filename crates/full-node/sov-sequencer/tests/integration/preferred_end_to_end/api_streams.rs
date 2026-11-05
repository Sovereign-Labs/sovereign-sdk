use super::*;

/// This test covers various endpoints that the sequencer provides for getting transactions and events.o
/// These tests are all squished into one because we have to do some relatively expensive setup (ie running a few hundred txs),
/// so we want to reuse that work for multiple endpoints.
#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_getters() {
    use sov_api_spec::types::TxInfoWithConfirmation;
    let (test_rollup, admin) = create_test_rollup_with(|b| {
        b.max_batch_execution_time_millis(1000) // Timeout the batch after 1 second of execution time.
    })
    .await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    let mut responses: Vec<TxInfoWithConfirmation> = Vec::new();
    let mut tx_number = 0;

    // The credential is available on the live stream, cache-backed historical stream, and
    // cache-backed REST response for a newly authenticated transaction.
    let expected_credential_id = admin
        .private_key
        .pub_key()
        .credential_id()
        .to_string()
        .parse::<types::Hash>()
        .unwrap();
    let mut live_tx_ws = test_rollup
        .api_client()
        .subscribe_to_txs(None)
        .await
        .unwrap();
    let tx = tx_set_value(&admin.private_key, tx_number, tx_number);
    let response = test_rollup
        .api_client()
        .send_raw_tx_to_sequencer_with_retry(&tx)
        .await
        .unwrap()
        .into_inner();
    let live_response = tokio::time::timeout(Duration::from_millis(500), live_tx_ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        live_response.credential_id.as_ref(),
        Some(&expected_credential_id)
    );

    let cached_response = test_rollup
        .api_client()
        .sequencer_get_tx(&response.id)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        cached_response.credential_id.as_ref(),
        Some(&expected_credential_id)
    );

    let mut cached_tx_ws = test_rollup
        .api_client()
        .subscribe_to_txs(Some(tx_number))
        .await
        .unwrap();
    let cached_stream_response =
        tokio::time::timeout(Duration::from_millis(500), cached_tx_ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    assert_eq!(
        cached_stream_response.credential_id.as_ref(),
        Some(&expected_credential_id)
    );
    responses.push(response);
    tx_number += 1;

    // Create a helper function to check that the tx endpoint responds with the expected txs.
    let check_responses = |starting_from: usize,
                           responses: Vec<TxInfoWithConfirmation>,
                           client: sov_api_spec::client::Client| async move {
        let mut tx_ws = client
            .subscribe_to_txs(Some(starting_from as u64))
            .await
            .unwrap();
        for (i, old_response) in responses.iter().enumerate().skip(starting_from) {
            let new_response = tokio::time::timeout(Duration::from_millis(500), tx_ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(
                new_response.id.to_string(),
                old_response.id.to_string(),
                "Mismatch at tx number {i}. Found {new_response:?} \nbut expected {old_response:?}",
            );
            assert_eq!(
                new_response.events, old_response.events,
                "Mismatch at tx {i}",
            );
        }
    };

    // Produce 30 blocks each containing 7 txs. After each block, check that the tx websocket responds with the expected txs. Starting from both zero and 5.
    // This should trigger any edge cases.
    for _ in 0..30 {
        for _ in 0..7 {
            let tx = tx_set_value(&admin.private_key, tx_number, tx_number);
            let resp = test_rollup
                .api_client()
                .send_raw_tx_to_sequencer_with_retry(&tx)
                .await
                .unwrap();
            responses.push(resp.into_inner());
            tx_number += 1;
        }
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
        check_responses(0, responses.clone(), test_rollup.api_client().clone()).await;
        check_responses(5, responses.clone(), test_rollup.api_client().clone()).await;
    }

    check_responses(0, responses.clone(), test_rollup.api_client().clone()).await;
    check_responses(5, responses.clone(), test_rollup.api_client().clone()).await;

    // Now, check the `get_tx` endpoint by iterating through all the txs we generated.
    // And fetching each tx by its id.
    for response in responses.iter() {
        let tx_response = test_rollup
            .api_client()
            .sequencer_get_tx(&response.id)
            .await
            .unwrap();
        let tx = tx_response.as_ref();
        assert_eq!(tx.id, response.id);
        assert_eq!(tx.events, response.events);
        assert_eq!(&tx.receipt, response.receipt.as_ref().unwrap());
        assert_eq!(&tx.tx_number, response.tx_number.as_ref().unwrap());
    }

    // Finally, test the "list events" endpoint by iterating through all the events we generated.
    let all_events = responses
        .into_iter()
        .flat_map(|response| response.events.into_iter())
        .collect::<Vec<_>>();
    let mut i = 0;
    let mut page = SequencerListEventsPage::First;
    let mut page_cursor: Option<String> = None;
    while i < all_events.len() {
        let response = test_rollup
            .api_client()
            .sequencer_list_events(
                Some(page),
                page_cursor.as_deref(),
                Some(std::num::NonZeroU32::new(9).unwrap()),
            ) // Use page size 9 because it's relatively prime to our number of events. This should trigger more edge cases
            .await
            .unwrap()
            .into_inner();
        let SequencerListEventsResponse { items, next_cursor } = response;
        for event in items {
            assert_eq!(event.number, i as u64);
            i += 1;
        }
        page = SequencerListEventsPage::Next;
        page_cursor = next_cursor;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_event_stream_filtering() {
    let (test_rollup, admin) = create_test_rollup_with(|b| {
        b.max_batch_execution_time_millis(1000) // Timeout the batch after 1 second of execution time.
    })
    .await;

    // Set up the rollup the usual way.
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut all_events = test_rollup
        .api_client()
        .subscribe_to_events()
        .await
        .unwrap();
    let mut value_setter_cpu_heavy_events = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("ValueSetter/RanCPUHeavyOperation")
        .await
        .unwrap();
    let mut bank_events = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();
    let mut value_setter_and_bank_events = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/*,ValueSetter/*")
        .await
        .unwrap();
    let tx = tx_set_value(&admin.private_key, 0, 1000);
    let _ = test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    assert!(
        tokio::time::timeout(Duration::from_secs(1), bank_events.next())
            .await
            .is_err(),
        "Bank event stream should not receive value setter txs"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), value_setter_cpu_heavy_events.next())
            .await
            .is_err(),
        "Value setter cpu heavy event stream should not receive bank events"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), all_events.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok(),
        "All event stream should receive all events"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), value_setter_and_bank_events.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok(),
        "Value setter and bank event stream should receive value setter events"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_event_stream_lag_message() {
    let events_channel_size = 5;

    let (genesis_params, admin) = create_genesis_params();
    let seq_da_address = genesis_params
        .runtime
        .sequencer_registry
        .sequencer_config
        .seq_da_address;
    let dir = tempdir_inside_codebase_dir();
    let builder = RollupBuilder::<RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        0,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir);
    })
    .with_preferred_seq_config(|seq| {
        seq.events_channel_size = events_channel_size;
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .set_persistent_da();

    // Set up the rollup the usual way.
    let test_rollup = builder.start().await.unwrap();
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Set a small TCP receive buffer (SO_RCVBUF) to cause actual TCP backpressure,
    // which should cause the server to block on send, leading to broadcast channel lag.
    // Note: The OS enforces a minimum buffer size (check /proc/sys/net/core/rmem_min).
    let mut events = test_rollup
        .api_client()
        .subscribe_to_events_with_socket_options(None, Some(256)) // Request tiny buffer
        .await
        .unwrap();

    let events_per_tx = 1000;
    let num_txs = 25;
    let emit_pattern: Vec<bool> = (0..events_per_tx).map(|i| i % 2 == 0).collect();

    for i in 0..num_txs {
        // Send a tx that emits many events to generate lots of data
        let tx = tx_event_emitter(&admin.private_key, emit_pattern.clone(), i as u64);
        let _ = test_rollup
            .api_client()
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        if i < 2 {
            for j in 0..events_per_tx {
                let event = events.next().await.unwrap().unwrap();
                assert_eq!(event.number, i as u64 * events_per_tx as u64 + j as u64);
            }
        }
    }

    // Now read events until we get a LAGGED error or the stream ends.
    // We expect a LAGGED error because we're generating more data than the
    // TCP buffers can hold while the client isn't reading. This causes the
    // server's WebSocket send to block, which causes the broadcast channel
    // to overflow.
    let mut received_lagged = false;
    let first_remaining_event = 2 * events_per_tx as u64;
    let mut event_count = first_remaining_event;

    while let Some(result) = events.next().await {
        match result {
            Ok(event) => {
                assert_eq!(
                    event.number, event_count,
                    "Event numbers should be sequential"
                );
                event_count += 1;
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("lagged") {
                    received_lagged = true;
                    assert!(
                        err_str
                            .contains(format!("\"disconnected_at\":{}", event_count - 1).as_str()),
                        "Expected \"disconnected_at\": {} in error message: {}",
                        event_count - 1,
                        err_str
                    );
                    break;
                } else {
                    panic!("Unexpected error: {e:?}");
                }
            }
        }
    }

    assert!(
        received_lagged,
        "Expected to receive a LAGGED notification, but received {event_count} events without lagging",
    );
}

// The sequencer controls emitting ledger slots over websocket
// this test ensures it correctly publishes all the expected slots
#[tokio::test(flavor = "multi_thread")]
async fn test_rollup_emits_all_slot_notifications() {
    let (test_rollup, _) = create_test_rollup().await;

    let nb_of_blocks = 10;
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(nb_of_blocks)
        .await
        .unwrap();

    let slot_timeout = std::time::Duration::from_millis(MAX_BATCH_EXECUTION_TIME_MILLIS);

    for _ in 0..nb_of_blocks {
        let slot = tokio::time::timeout(slot_timeout, slot_subscription.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tracing::info!("received slot {}", slot.number);
    }

    test_rollup.shutdown().await.unwrap();
}

use super::*;

const LAST_EVENT_NUMBER_HEADER: &str = "x-sov-last-event-number";
const EVENT_EPOCH_HEADER: &str = "x-sov-event-epoch";

/// Reads the ValueSetter's value together with the event number the reply says it reflects.
///
/// The value is `None` until something has been set.
async fn read_value_and_last_event_number(
    test_rollup: &TestRollup<TestBlueprint>,
    query: &str,
) -> (Option<u64>, Option<u64>) {
    let (header, body) = test_rollup
        .client
        .http_get_with_header(
            &format!("/modules/value-setter/state/value{query}"),
            LAST_EVENT_NUMBER_HEADER,
        )
        .await
        .unwrap();

    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        body.get("value").is_some(),
        "Expected a state value reply, got {body:?}"
    );

    (
        header.map(|header| header.parse().unwrap()),
        body["value"].as_u64(),
    )
}

/// Reads the event epoch the ValueSetter reply is stamped with.
async fn read_event_epoch(test_rollup: &TestRollup<TestBlueprint>) -> Option<String> {
    test_rollup
        .client
        .http_get_with_header("/modules/value-setter/state/value", EVENT_EPOCH_HEADER)
        .await
        .unwrap()
        .0
}

/// A state reply must name the last event it reflects, so that a client holding both a snapshot
/// and the event stream knows exactly which events are still to be applied.
#[tokio::test(flavor = "multi_thread")]
async fn state_reads_report_the_last_event_number_they_reflect() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut events = test_rollup
        .api_client()
        .subscribe_to_events()
        .await
        .unwrap();

    let (before, _) = read_value_and_last_event_number(&test_rollup, "").await;

    // Set a value. The transaction emits events, so the frontier must advance past them. The
    // sequencer applies the transaction's writes and advances the frontier before it answers
    // the submitter, so by the time this returns a state read must show both.
    let value_to_set = 0x1234_5678;
    let response = test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(tx_set_value(&admin.private_key, 0, value_to_set)),
        })
        .await
        .unwrap()
        .into_inner();
    let tx_event_numbers: Vec<u64> = response.events.iter().map(|event| event.number).collect();
    let last_tx_event_number = tx_event_numbers
        .last()
        .copied()
        .expect("Setting a value should emit at least one event");

    let (reported, value) = read_value_and_last_event_number(&test_rollup, "").await;
    assert_eq!(value, Some(value_to_set));
    assert_eq!(
        reported,
        Some(last_tx_event_number),
        "A state read that already shows the transaction's writes must report that \
         transaction's last event number; reporting less would make a client re-apply events \
         it has already seen, reporting more would make it skip some"
    );
    assert!(
        before < reported,
        "The reported event number should advance as events are emitted; it was {before:?} \
         before the transaction and {reported:?} after"
    );

    // The numbers the stream hands out are the ones the frontier was measured against.
    let mut streamed = Vec::with_capacity(tx_event_numbers.len());
    for _ in &tx_event_numbers {
        let event = tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .expect("Timed out waiting for the transaction's events on the stream")
            .unwrap()
            .unwrap();
        streamed.push(event.number);
    }
    assert_eq!(
        streamed, tx_event_numbers,
        "The stream must hand out the same numbers the submitter was told about"
    );
}

/// The live frontier describes the sequencer's head, so it must not be stamped onto a reply
/// that deliberately reads older state.
#[tokio::test(flavor = "multi_thread")]
async fn historical_state_reads_report_no_last_event_number() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut events = test_rollup
        .api_client()
        .subscribe_to_events()
        .await
        .unwrap();

    // Emit an event, so that there is a frontier to report in the first place.
    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(tx_set_value(&admin.private_key, 0, 0x1234_5678)),
        })
        .await
        .unwrap();

    // The sequencer advances the frontier before it streams the transaction's events, so once
    // one arrives a latest-state read must report it.
    tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("The transaction's events should be streamed")
        .expect("The event stream should not end")
        .unwrap();
    let (latest, _) = read_value_and_last_event_number(&test_rollup, "").await;
    assert!(
        latest.is_some(),
        "A latest-state read should report an event number once the rollup has emitted events"
    );

    let (historical, _) = read_value_and_last_event_number(&test_rollup, "?rollup_height=1").await;
    assert_eq!(
        historical, None,
        "A historical read must not be stamped with the live event number"
    );
}

/// Normal operation extends the numbering rather than reissuing it, so the epoch must hold
/// still. A client that resynchronised on every batch would be useless.
#[tokio::test(flavor = "multi_thread")]
async fn the_event_epoch_is_stable_while_the_sequencer_makes_progress() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut epochs = Vec::new();
    let mut numbers = Vec::new();

    for generation in 0..3u64 {
        test_rollup
            .api_client()
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(tx_set_value(
                    &admin.private_key,
                    generation,
                    0x1234_5678 + generation,
                )),
            })
            .await
            .unwrap();

        // Close the batch and let the node catch up, so the reads straddle a slot boundary and
        // exercise the republish paths rather than just `apply_tx_changes`.
        test_rollup.force_close_batch().await.unwrap();
        test_rollup.produce_enough_finalized_slots().await;
        test_rollup.wait_for_node_synced().await.unwrap();

        if let Some(epoch) = read_event_epoch(&test_rollup).await {
            epochs.push(epoch);
            numbers.push(read_value_and_last_event_number(&test_rollup, "").await.0);
        }
    }

    assert!(
        epochs.len() >= 2,
        "Expected several reads carrying an epoch, got {epochs:?}"
    );
    assert_eq!(
        epochs
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1,
        "The epoch changed without a rollback, which would force clients to resynchronise for \
         no reason: {epochs:?}"
    );
    assert!(
        numbers.windows(2).all(|w| w[0] <= w[1]),
        "Within one epoch the event number must only move forward, got {numbers:?}"
    );
}

/// The epoch on the state reply is only half the story: a client applying streamed events needs
/// to know they belong to the same run of numbering its snapshot came from. So the events must
/// carry it too, and the two must agree.
#[tokio::test(flavor = "multi_thread")]
async fn streamed_events_carry_the_same_epoch_as_the_state_reply() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut events = test_rollup
        .api_client()
        .subscribe_to_events()
        .await
        .unwrap();

    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(tx_set_value(&admin.private_key, 0, 0x1234_5678)),
        })
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("Timed out waiting for an event")
        .expect("Event stream ended")
        .expect("Event stream errored");

    let epoch_on_event = event
        .epoch
        .clone()
        .expect("A soft-confirmed event must carry the epoch its number was handed out in");

    let epoch_on_state = read_event_epoch(&test_rollup)
        .await
        .expect("A latest-state read should report an epoch once events exist");

    assert_eq!(
        epoch_on_event, epoch_on_state,
        "The stream and the state reply must agree on the epoch, otherwise a client cannot tell \
         whether the events it is applying belong to the snapshot it is holding"
    );
}

/// The submitter's own reply is sent before the transaction reaches the event stream, so its
/// events are labelled separately from the stream's. They must still carry the same epoch.
#[tokio::test(flavor = "multi_thread")]
async fn submitted_tx_reply_events_carry_the_same_epoch_as_the_state_reply() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let response = test_rollup
        .api_client()
        .send_raw_tx_to_sequencer(&tx_set_value(&admin.private_key, 0, 0x1234_5678))
        .await
        .unwrap();
    let event = response
        .events
        .first()
        .expect("Setting a value emits an event");

    let epoch_on_state = read_event_epoch(&test_rollup)
        .await
        .expect("A latest-state read should report an epoch once events exist");

    assert_eq!(
        event.epoch.as_deref(),
        Some(epoch_on_state.as_str()),
        "The submitter's reply must carry the epoch its events were numbered in, like the stream"
    );
}

/// The node's ledger API has no epoch, so the events it serves carry none; an absent epoch
/// counts as zero. Only the sequencer labels the ledger-backed copies it serves itself.
#[tokio::test(flavor = "multi_thread")]
async fn committed_ledger_events_carry_no_epoch() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(tx_set_value(&admin.private_key, 0, 0x1234_5678)),
        })
        .await
        .unwrap();

    // Drive the transaction all the way into the ledger.
    test_rollup.force_close_batch().await.unwrap();
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_node_synced().await.unwrap();

    let event = test_rollup
        .api_client()
        .get_latest_event()
        .await
        .expect("The rollup should have committed at least one event")
        .into_inner();

    assert_eq!(
        event.epoch, None,
        "The node's ledger API knows no epoch, so it must not invent one"
    );
}

/// The standard sequencer publishes node-committed state only, so its replies must name the
/// node's last committed event, under the constant zero epoch.
#[tokio::test(flavor = "multi_thread")]
async fn standard_sequencer_state_reads_report_the_nodes_last_event() {
    let dir = tempdir_inside_codebase_dir();
    let (mut genesis_params, admin) = create_genesis_params();
    let sequencer_config = &mut genesis_params.runtime.sequencer_registry.sequencer_config;
    sequencer_config.is_preferred_sequencer = false;
    let seq_da_address = sequencer_config.seq_da_address;

    let test_rollup = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        0,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir);
        c.rollup_prover_config = RollupProverConfig::Disabled;
        c.automatic_batch_production = true;
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .with_standard_sequencer()
    .start()
    .await
    .unwrap();

    let mut slots = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(tx_set_value(&admin.private_key, 0, 0x1234_5678)),
        })
        .await
        .unwrap();

    // The sequencer submits its batch when it sees the next slot, and the node executes it in
    // the slot after; produce slots until the value is committed.
    let mut last_event_number = None;
    for _ in 0..10 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        next_within(&mut slots, NOTIFICATION_TIMEOUT).await.unwrap();
        let (number, value) = read_value_and_last_event_number(&test_rollup, "").await;
        if value == Some(0x1234_5678) {
            last_event_number = number;
            break;
        }
    }

    let latest_event = test_rollup
        .api_client()
        .get_latest_event()
        .await
        .expect("The value-setting transaction should have committed an event")
        .into_inner();
    assert_eq!(
        last_event_number,
        Some(latest_event.number),
        "A reply from the standard sequencer must name the node's last committed event"
    );
    assert_eq!(
        read_event_epoch(&test_rollup).await,
        Some(sov_modules_api::EventEpoch::ZERO.to_string()),
        "A publisher that never reissues numbers reports the zero epoch, so its clients never \
         have to resynchronise over an epoch change"
    );
}

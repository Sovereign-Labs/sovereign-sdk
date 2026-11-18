use super::*;

/// Test what happens when the sequencer fills up its gas limit. This tests that...
/// 1. Transactions which would exceed the gas limit are rejected.
/// 2. Really large transactions don't cause the sequencer to produce a batch too early. It only considers a batch full once we've used 95% of the gas.
/// 3. The sequencer does produce a new batch once the current one gets close the gas limit
#[tokio::test(flavor = "multi_thread")]
async fn sequencer_filled_up_block() {
    let mut genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let max_exec_gas_per_tx = GasUnit::from(config_value!("MAX_SEQUENCER_EXEC_GAS_PER_TX"));

    let gas_limit = max_exec_gas_per_tx;
    let gas_limit = gas_limit.checked_scalar_product(100).unwrap();
    // The sequencer only gets 90% of the overall gas limit, so we need to set the slot gas limit to 10/9ths of what we
    // want the sequencer to have.
    let gas_limit_array = gas_limit
        .as_ref()
        .iter()
        .map(|x| (x * 10) / 9)
        .collect::<Vec<_>>();
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_BLOCK_GAS_LIMIT",
        format!("{gas_limit_array:?}"),
    );
    // Set very high initial balance for the admin.
    genesis_config.additional_accounts_mut()[0].available_gas_balance =
        Amount::MAX.saturating_div(Amount::new(2));

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
    .finalization_blocks(0)
    .start()
    .await;

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    // This means finalized blocks + 2 as buffer.
    test_rollup
        .tenderly_produce_blocks((TEST_FINALIZATION_BLOCKS + 2) as usize)
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    let client = test_rollup.api_client().clone();

    {
        let gas_to_charge = gas_limit
            .checked_scalar_product(9)
            .unwrap()
            .scalar_division(10);

        // Produce a transaction that uses 90% of the slot gas limit.
        // This should be accepted.
        let tx = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            0,
            7,
            Some(gas_to_charge),
            Amount::MAX.saturating_div(Amount::new(4)),
        );
        client.send_raw_tx_to_sequencer(&tx).await.unwrap();

        // Produce a second huge transaction
        // This should fail with "Out of Gas" because:
        //  - the sequencer is below its 95% gas usage threshold so no new batch will have been started.
        //  - there's not nearly enough slot gas limit left to cover this tx
        let tx_2 = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            1,
            7,
            Some(gas_to_charge),
            Amount::MAX.saturating_div(Amount::new(4)),
        );

        let err = client.send_raw_tx_to_sequencer(&tx_2).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("The gas to charge is greater than the funds available in the meter"),
            "Expected Out of Gas error, got: {err}"
        );

        // Produce a third transaction that uses 5% of the gas limit.
        // This should be accepted and will cause the sequencer to close out its current batch since usage should now pass 95%.
        let small_gas_amount = gas_limit.scalar_division(20);
        let tx_3 = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            1,
            7,
            Some(small_gas_amount),
            Amount::MAX.saturating_div(Amount::new(4)),
        );
        client.send_raw_tx_to_sequencer(&tx_3).await.unwrap();

        // Produce another huge transaction
        // This should be accepted because the sequencer starts a new batch.
        let tx_4 = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            1,
            7,
            Some(gas_to_charge),
            Amount::MAX.saturating_div(Amount::new(4)),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx_4),
            })
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn max_batch_size() {
    let max_batch_size = 1024;
    let (test_rollup, admin) = create_test_rollup_with(|b| {
        b.max_batch_size_bytes(max_batch_size)
            .finalization_blocks(0)
            .block_producing_config(TEST_DEFAULT_MOCK_DA_ON_SUBMIT)
    })
    .await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.expect(
        "Sequencer is not ready after initial empty blocks for creation of finalized slots",
    );

    let validate_expected_error = |resp: Error<types::ApiError>| {
        let error_str = resp.to_string();
        let pattern1 = "Transaction cannot be included in the batch";
        let pattern2 = "The transaction is too large";
        assert!(
            error_str.contains(pattern1),
            "pattern: {pattern1} not found, actual error: {error_str}"
        );
        assert!(
            error_str.contains(pattern2),
            "pattern: {pattern2} not found, actual error: {error_str}"
        );
    };

    let client = test_rollup.api_client().clone();

    // The transaction is rejected because it is too large.
    {
        let tx = tx_set_many_values(&admin.private_key, 0, vec![0; 1024]);
        let resp = client.send_raw_tx_to_sequencer(&tx).await.unwrap_err();
        validate_expected_error(resp);
    }

    test_rollup.pause_preferred_batches().await;
    // The first and third transactions are processed, but the second and fourth are too large to be included in the batch.
    {
        let tx = tx_set_many_values(&admin.private_key, 0, vec![0; 128]);
        let _ = client.send_raw_tx_to_sequencer(&tx).await.unwrap();

        let tx = tx_set_many_values(&admin.private_key, 1, vec![0; 1024]);
        let resp = client.send_raw_tx_to_sequencer(&tx).await.unwrap_err();
        validate_expected_error(resp);

        // Fully baked txs include auth wrapper + sequencing metadata + borsh overhead.
        // 480 bytes keeps two medium txs below the 99% comfortable size limit for a 1024 batch.
        let tx = tx_set_many_values(&admin.private_key, 1, vec![0; 480]);
        let _ = client.send_raw_tx_to_sequencer(&tx).await.unwrap();

        let tx = tx_set_many_values(&admin.private_key, 2, vec![1; 480]);
        let resp = client.send_raw_tx_to_sequencer(&tx).await.unwrap_err();
        validate_expected_error(resp);
    }
    test_rollup.force_close_batch().await.unwrap();
    // Producing couple more block to avoid lack of finalized
    test_rollup.resume_preferred_batches().await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup
        .wait_for_node_synced()
        .await
        .expect("Node is not synced after big txs");
    test_rollup
        .wait_for_sequencer_ready()
        .await
        .expect("Sequencer is not ready before final tx");
    test_rollup.pause_preferred_batches().await;
    // Once we start creating a fresh batch, we can insert a transaction that was previously rejected.
    {
        let tx = tx_set_many_values(&admin.private_key, 2, vec![1; 480]);
        let _ = client.send_raw_tx_to_sequencer(&tx).await.unwrap();
    }
}

/// This test checks that the sequencer closes its current batch when the tx **execution** time exceeds its target.
/// Scenario. Percentage is from max execution of the batch parameter
/// | tx# |    sleep time | batch# | exec time | wall clock |
/// | --- | ------------- | ------ | --------- | ---------- |
/// |  1  |  > batch_time |      0 |    > 100% |            |
/// |  2  |          50%  |      1 |      150% |       150% |
/// |  3  |      50% + bt |      1 |    > 200% |     > 200% |
/// |  4  |           33% |      2 |      233% |       233% |
/// | --- |           50% |      2 |      233% |       283% |
/// |  5  |           33% |      2 |      266% |       313% |
/// |  6  |       33% +bt |      2 |    > 300% |     > 346% |
/// This way this test can validate that only **execution time** of tx counts
#[tokio::test(flavor = "multi_thread")]
async fn flaky_max_batch_execution_time() {
    // Timeout the batch after 2 seconds of **execution time**.
    let max_batch_exec_time_millis = 2000;
    let approx_batch_time_millis = 400;
    let (test_rollup, admin) = create_test_rollup_with(|b| {
        b.max_batch_execution_time_millis(max_batch_exec_time_millis)
            .block_producing_config(TEST_DEFAULT_MOCK_DA_ON_SUBMIT)
    })
    .await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    let client = test_rollup.api_client().clone();
    let mut slot_subscription = client
        .subscribe_slots_with_children(IncludeChildren::new(true))
        .await
        .unwrap();

    // Tx1: Takes longer than the batch timeout, causing batch 0 to close
    let tx_1 = tx_set_value_and_sleep(
        &admin.private_key,
        0,
        1,
        max_batch_exec_time_millis + approx_batch_time_millis,
    );
    // After tx 2, batch 1: execution time: 50%
    let tx_2 = tx_set_value_and_sleep(&admin.private_key, 0, 2, max_batch_exec_time_millis / 2);
    // After tx 3, batch 1: execution time: 100% + cushion. Close batch 1 and create batch 2
    let tx_3 = tx_set_value_and_sleep(
        &admin.private_key,
        1,
        3,
        (max_batch_exec_time_millis / 2) + approx_batch_time_millis,
    );
    // After tx 4, batch 2: execution time: 33%
    let tx_4 = tx_set_value_and_sleep(&admin.private_key, 1, 4, max_batch_exec_time_millis / 3);
    // After tx 5, batch 2: execution time: 66%
    let tx_5 = tx_set_value_and_sleep(&admin.private_key, 2, 5, max_batch_exec_time_millis / 3);
    // After tx 6, batch 2: execution time: 100% + cushion. Close batch 2
    let tx_6 = tx_set_value_and_sleep(
        &admin.private_key,
        3,
        6,
        (max_batch_exec_time_millis / 3) + approx_batch_time_millis,
    );

    // txs per slot, where each slot will have only single batch, because we only produce block on submit
    let expected_tx_placement = vec![1, 2, 3];

    let _ = client.send_raw_tx_to_sequencer(&tx_1).await.unwrap();
    let _ = client.send_raw_tx_to_sequencer(&tx_2).await.unwrap();
    let _ = client.send_raw_tx_to_sequencer(&tx_3).await.unwrap();
    // Wait until tx_2 and tx_3 have accumulated execution time in the current batch.
    // This creates the intended boundary before sending tx_4..tx_6.
    tokio::time::sleep(std::time::Duration::from_millis(
        max_batch_exec_time_millis / 2,
    ))
    .await;
    let _ = client.send_raw_tx_to_sequencer(&tx_4).await.unwrap();
    let _ = client.send_raw_tx_to_sequencer(&tx_5).await.unwrap();
    let _ = client.send_raw_tx_to_sequencer(&tx_6).await.unwrap();

    let wait_slot_timeout = std::time::Duration::from_millis(max_batch_exec_time_millis) * 4;
    let mut slot_summaries = Vec::new();

    #[derive(Debug, PartialEq)]
    struct ShortSlotSummary {
        number: u64,
        batches: usize,
        txs: usize,
    }

    let mut total_txs = 0;
    while let Ok(slot_response) =
        tokio::time::timeout(wait_slot_timeout, slot_subscription.next()).await
    {
        let slot = slot_response.unwrap().unwrap();
        let txs: usize = slot.batches.iter().map(|b| b.txs.len()).sum();
        total_txs += txs;
        let summary = ShortSlotSummary {
            number: slot.number,
            batches: slot.batches.len(),
            txs,
        };
        // Rollup can produce empty batch for moving visible slot forward, so we skip batches with no txs.
        if txs > 0 {
            slot_summaries.push(summary);
        }
    }

    assert_eq!(
        total_txs, 6,
        "number of processed txs (left) does not match number of sent (right)"
    );
    for (expected_txs, summary) in expected_tx_placement
        .into_iter()
        .zip(slot_summaries.into_iter())
    {
        assert_eq!(
            1, summary.batches,
            "Wrong number of batches in slot {}",
            summary.number
        );
        assert_eq!(
            expected_txs, summary.txs,
            "Wrong number of txs in slot {}",
            summary.number
        );
    }
    tokio::time::timeout(TEST_NORMAL_SHUTDOWN_TIMEOUT, test_rollup.shutdown())
        .await
        .unwrap()
        .unwrap();
}

/// Test starts rollup, submits transaction, blocks blob submission on mock da.
/// Then submit many transactions, until we hit backpressure
/// Unblock MockDa submission, produce some blocks and let sequencer recover
/// Send transaction again and it should be accepted
#[tokio::test(flavor = "multi_thread")]
async fn sequencer_back_pressure() {
    let max_batch_execution_time_millis = 1_000;
    let (test_rollup, admin) = create_test_rollup_with(|b| {
        b.max_batch_execution_time_millis(max_batch_execution_time_millis)
    })
    .await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let (slot_tx, mut slot_rx) = tokio::sync::mpsc::channel(100);
    // Keep receiving from the slot subscription in the background to avoid timeouts
    let mut slot_subscription = test_rollup.client.client.subscribe_slots().await.unwrap();
    tokio::spawn(async move {
        while let Some(slot) = slot_subscription.next().await {
            slot_tx.send(slot).await.unwrap();
        }
    });
    let client = test_rollup.api_client().clone();

    let mut generation = 0;
    print_blobs_at_head(&test_rollup.da_service).await;
    // Pause block submission and produce some pending blocks.
    {
        let expected_patterns = [
            "The sequencer is temporarily overloaded",
            "The preferred sequencer is recovering from downtime and cannot provide soft-confirmations at this time; No new transactions can be accepted, try again later",
            "The sequencer is waiting for the blob sender to be ready"
        ];
        test_rollup.da_service.set_blob_submission_pause().await;

        let mut bytes_submitted = 0;
        let max_batch_size = TEST_MAX_BATCH_SIZE * 100 / 99;
        let max_total = max_batch_size * TEST_MAX_CONCURRENT_BATCH_BLOBS;
        let mut has_hit_backpressure = false;
        while bytes_submitted < max_total {
            let tx = tx_set_value(&admin.private_key, generation, generation + 10);
            let result = client.send_raw_tx_to_sequencer(&tx).await;
            match result {
                Ok(_) => {
                    // Doing larger increase to make sure we won't hit Too many transactions for credential_id
                    // Switching to nonce will fix that, but this is for later
                    generation += 1000;
                    bytes_submitted += tx.len();
                }
                Err(err) => {
                    let error_string = err.to_string();
                    let found = expected_patterns
                        .iter()
                        .find(|pattern| error_string.contains(**pattern));
                    if found.is_some() {
                        has_hit_backpressure = true;
                        break;
                    }
                    panic!("Unexpected error from sequencer: {error_string}");
                }
            }
        }
        assert!(
            has_hit_backpressure,
            "Test hasn't hit backpressure after {generation} txs and total bytes {bytes_submitted} submitted"
        );
        tracing::warn!("BACKPRESSURE GOT, RESUMING BLOB SUBMISSION AFTER {generation} txs send");
        test_rollup.da_service.resume_blob_submission().await;
    }
    print_blobs_at_head(&test_rollup.da_service).await;
    test_rollup.da_service.produce_block_now().await.unwrap();
    print_blobs_at_head(&test_rollup.da_service).await;

    // Choose this number arbitrarily.
    let end_padding_blocks = 30;
    for _ in 0..end_padding_blocks {
        test_rollup.da_service.produce_block_now().await.unwrap();
        // Give the node time to ingest each padding block so debug output reflects progress.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        print_blobs_at_head(&test_rollup.da_service).await;
    }

    for _ in 0..(end_padding_blocks - 10) {
        let _slot = slot_rx.recv().await.unwrap().unwrap();
    }

    test_rollup.wait_for_node_synced().await.unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let tx = tx_set_value(&admin.private_key, generation + 1, 9);
    client
        .send_raw_tx_to_sequencer(&tx)
        .await
        .expect("Sequencer failed to accept tx after recovery");
}

async fn print_blobs_at_head(da_service: &StorableMockDaService) {
    let height = da_service.get_head_block_header().await.unwrap().height;
    let block = da_service.get_block_at(height).await.unwrap();

    let batches = block.batch_blobs.len();
    let proofs = block.batch_blobs.len();
    println!("H={height}; BATCHES={batches} PROOFS={proofs}");
}

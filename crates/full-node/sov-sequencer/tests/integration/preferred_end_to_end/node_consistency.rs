use super::*;

/// Ensure that we use the correct visible slot number when replaying transactions after a call to `update_state` in the sequencer.
/// The key thing that this test does is to execute the same transaction 3 times - once in the sequencer via `accept_tx`, once via `update_state`
/// and once in the node. Everything else is implementation details.
#[tokio::test(flavor = "multi_thread")]
async fn replay_uses_correct_visible_slot_number() {
    let tx_builder = |key| tx_assert_visible_slot_number(&key, 0, 2);
    do_manual_block_production_test(tx_builder, |_| async {}).await;
}

/// This test checks that the visible hash of the rollup block is the same in the node and the sequencer.
///
/// This test currently works by running a loop of...
/// - Send a transaction to trigger the sequencer to start a batch.
/// - Query the current state root from the sequencer.
/// - Send a transaction that asserts the correct state root. Ensure it is accepted
/// - Produce a block, triggering the sequencer to close out its current batch and post it on DA
/// - Check that the state root assertion succeeded on the node as well.
#[tokio::test(flavor = "multi_thread")]
async fn visible_hashes_match_across_node_and_sequencer() {
    const FINALIZATION_BLOCKS: u32 = 0;
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
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

    let dir = Arc::new(tempfile::tempdir().unwrap());

    let da_layer = Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(FINALIZATION_BLOCKS)
            .await
            .unwrap(),
    ));
    let test_rollup = {
        let sequencer_addr = genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address;
        RollupBuilder::<TestBlueprint>::new(
            GenesisSource::CustomParams(genesis_params),
            BlockProducingConfig::Manual, // Use manual block production to be sure that the changes are happening in the sequencer only, not the node.
            FINALIZATION_BLOCKS,
        )
        .set_config(|c| {
            c.rollup_prover_config = RollupProverConfig::Disabled;
            c.storage = StoragePath::Tmp(dir);
        })
        .set_da_config(|c| {
            c.sender_address = sequencer_addr;
            c.da_layer = Some(da_layer.clone());
        })
        .start()
        .await
        .unwrap()
    };

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut state_update_subscription = test_rollup.subscribe_state_updates().await.unwrap();
    let mut slot_subscription = test_rollup
        .api_client()
        .subscribe_slots_with_children(IncludeChildren::new(true))
        .await
        .unwrap();

    #[derive(Debug, serde::Deserialize)]
    struct StateRootValue {
        root_hashes: Vec<u8>,
    }
    #[derive(Debug, serde::Deserialize)]
    struct StateRootResponse {
        value: StateRootValue,
    }
    async fn get_state_root(test_rollup: &TestRollup<TestBlueprint>) -> StateRootValue {
        let state_root_url = format!(
            "{}/modules/hooks-count/state/latest-state-root/",
            test_rollup.api_client().baseurl()
        );
        let response = test_rollup
            .api_client()
            .client()
            .get(state_root_url)
            .send()
            .await
            .unwrap();
        let response = response
            .json::<StateRootResponse>()
            .await
            .expect("Hooks must have run");
        response.value
    }
    // Produce some empty blocks to ensure that the sequencer has a batch in progress.
    da_layer.write().await.produce_block().await.unwrap();
    next_within(&mut slot_subscription, NOTIFICATION_TIMEOUT)
        .await
        .unwrap();
    next_within(&mut state_update_subscription, NOTIFICATION_TIMEOUT)
        .await
        .unwrap();
    da_layer.write().await.produce_block().await.unwrap();
    next_within(&mut slot_subscription, NOTIFICATION_TIMEOUT)
        .await
        .unwrap();
    next_within(&mut state_update_subscription, NOTIFICATION_TIMEOUT)
        .await
        .unwrap();

    // Run a few rounds of checking the state root to be extra sure nothing gets screwed up over time.
    let mut current_nonce = 0;
    for i in 0..10 {
        // Send a transaction to ensure that the sequencer has a batch in progress. This is necessary
        // because we start a new rollup block (with a new visible hash) each time we start a batch.
        let tx = tx_set_value(&admin.private_key, current_nonce, i + 100).clone();
        test_rollup
            .api_client()
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        current_nonce += 1;

        // Query the current state root from the node.
        let root = get_state_root(&test_rollup).await;
        tracing::info!(
            "Sending assert state root tx: {}",
            hex::encode(&root.root_hashes)
        );

        // Send a transaction that asserts the correct state root.
        let tx = tx_assert_state_root(&admin.private_key, current_nonce, root.root_hashes.clone())
            .clone();
        test_rollup
            .api_client()
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        current_nonce += 1;

        // Produce a block. This will trigger the sequencer to close out its current batch and start a new one.
        test_rollup.force_close_batch().await.unwrap();
        da_layer.write().await.produce_block().await.unwrap();
        let slot = next_within(&mut slot_subscription, NOTIFICATION_TIMEOUT)
            .await
            .unwrap();
        if !slot.batches.is_empty() && !slot.batches[0].txs.is_empty() {
            // Assert that the second transaction in the batch (the one that asserts the state root) succeeded.
            assert_eq!(
                slot.batches[0].txs[1].receipt.result,
                TxReceiptResult::Successful
            );
        }
        next_within(&mut state_update_subscription, NOTIFICATION_TIMEOUT)
            .await
            .unwrap();
    }
}

/// This test checks that state changes from the begin/end slot and finalize hooks are visible via the sequencer's REST API.
///
/// It works by producing several batches in the sequencer (causing the hooks to be run) without every publishing those batches
/// to DA (ensuring that the state changes are not visible to the node), then querying the state via the REST API.
// TODO(@neysofu): unflaky it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Broken by removal of /batches endpoint, will be testable again when we add node/sequencer state comparison support"]
async fn flaky_test_hooks_state_is_visible() {
    const FINALIZATION_BLOCKS: u32 = 3;
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
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

    let dir = Arc::new(tempfile::tempdir().unwrap());

    let da_layer = Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(FINALIZATION_BLOCKS)
            .await
            .unwrap(),
    ));
    let test_rollup = {
        let sequencer_addr = genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address;
        RollupBuilder::<TestBlueprint>::new(
            GenesisSource::CustomParams(genesis_params),
            BlockProducingConfig::Manual, // Use manual block production to be sure that the changes are happening in the sequencer only, not the node.
            FINALIZATION_BLOCKS,
        )
        .set_config(|c| {
            c.automatic_batch_production = false;
            c.rollup_prover_config = RollupProverConfig::Disabled;
            c.storage = StoragePath::Tmp(dir);
        })
        .set_da_config(|c| {
            c.sender_address = sequencer_addr;
            c.da_layer = Some(da_layer.clone());
        })
        .start()
        .await
        .unwrap()
    };

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // By the time we query here, the sequencer has *started* the next slot, so it has run the begin slot hook a second time.
    let client = NodeClient::new(test_rollup.api_client().baseurl())
        .await
        .unwrap();

    let query_hook_counter = |hook_name: &'static str| async {
        let hook_name = hook_name.to_string();
        client
            .query_rest_endpoint::<ValueResponse>(&format!(
                "/modules/hooks-count/state/{hook_name}-hook-count"
            ))
            .await
            .unwrap()
            .value
    };

    let begin_slot_count = query_hook_counter("begin-rollup-block").await;
    assert_eq!(begin_slot_count, 0);
    let begin_slot_count = query_hook_counter("end-rollup-block").await;
    assert_eq!(begin_slot_count, 0);

    {
        let txs = generate_txs(admin.private_key.clone()).clone();
        for tx in txs {
            client
                .client
                .send_raw_tx_to_sequencer(&tx.raw_tx)
                .await
                .unwrap();
        }
    }

    let begin_slot_count = query_hook_counter("begin-rollup-block").await;
    assert_eq!(begin_slot_count, 1);

    //  since we haven't finished building this batch, the end slot hook hasn't been run - so its value is still 0
    let end_slot_count = query_hook_counter("end-rollup-block").await;
    assert_eq!(end_slot_count, 0);
    // was run once during genesis
    let finalize_count = query_hook_counter("finalize").await;
    assert_eq!(finalize_count, 1);

    test_rollup
        .da_service
        .produce_n_blocks_now(10)
        .await
        .unwrap();
    test_rollup.wait_for_node_synced().await.unwrap();

    let begin_slot_count = query_hook_counter("begin-rollup-block").await;
    assert_eq!(begin_slot_count, 1);
    let end_slot_count = query_hook_counter("end-rollup-block").await;
    assert_eq!(end_slot_count, 1);
    let finalize_count = query_hook_counter("finalize").await;
    assert_eq!(finalize_count, 2);
}

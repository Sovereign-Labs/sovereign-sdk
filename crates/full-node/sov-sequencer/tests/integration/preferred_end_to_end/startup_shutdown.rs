use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn rollup_shuts_down_if_blob_sender_fails() {
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 9);

    tracing::warn!("accepting tx");
    client.send_raw_tx_to_sequencer(&tx).await.unwrap();

    test_rollup.da_service.set_fail_send_blob();
    tracing::warn!("setting fail send blob");
    test_rollup.force_close_batch().await.unwrap();
    tracing::warn!("force closed batch");

    test_rollup
        .wait_for_rollup_to_shutdown(TEST_NORMAL_SHUTDOWN_TIMEOUT)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rollup_shuts_down_if_blob_processing_timeouts() {
    let blob_processing_timeout_secs = 1;
    let (test_rollup, admin) =
        create_test_rollup_with(|b| b.blob_processing_timeout_secs(blob_processing_timeout_secs))
            .await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Send a transaction to ensure a batch is created.
    let tx = tx_set_value(&admin.private_key, 0, 9);
    test_rollup
        .api_client()
        .send_raw_tx_to_sequencer(&tx)
        .await
        .unwrap();

    // Close the batch and submit it. It won't be received because we don't create any more blocks
    test_rollup.force_close_batch().await.unwrap();

    test_rollup
        .wait_for_rollup_to_shutdown(
            TEST_NORMAL_SHUTDOWN_TIMEOUT
                + std::time::Duration::from_secs(blob_processing_timeout_secs),
        )
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rollup_shuts_down_if_panic_is_triggered() {
    let (test_rollup, admin) =
        create_test_rollup_with(|b| b.blob_processing_timeout_secs(60)).await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();

    // Send a test transaction to make sure everything is working.
    let tx = tx_set_value(&admin.private_key, 0, 9);
    client
        .send_raw_tx_to_sequencer_with_retry(&tx)
        .await
        .unwrap();

    // Cause a panic in the sequencer.
    let tx = tx_panic(&admin.private_key, 1);
    client.send_raw_tx_to_sequencer(&tx).await.unwrap_err();

    // Ensure that the sequencer rejects subsequent transactions.
    let tx = tx_set_value(&admin.private_key, 2, 5);
    client.send_raw_tx_to_sequencer(&tx).await.unwrap_err();

    // Ensure that the sequencer shuts down promptly
    test_rollup
        .wait_for_rollup_to_shutdown_with_result(TEST_NORMAL_SHUTDOWN_TIMEOUT * 2)
        .await
        .expect_err("Rollup should have shut down with an error due to panicking");
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "DA address mismatch")]
async fn preferred_sequencer_fails_on_da_address_mismatch() {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let genesis_params = GenesisParams {
        runtime: <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: sov_modules_api::Address::from([0; 28]),
            },
            (),
            PaymasterConfig::default(),
            (),
            (),
        ),
    };

    RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        0,
    )
    .set_config(|conf| {
        conf.sequencer_config = SequencerKindConfig::Preferred(Default::default());
    })
    .start()
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "No preferred sequencer is registered in the sequencer registry")]
async fn preferred_sequencer_fails_when_no_preferred_sequencer_in_registry() {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let seq_da_address = HighLevelOptimisticGenesisConfig::<TestSpec>::sequencer_da_addr();
    let mut minimal: MinimalOptimisticGenesisConfig<TestSpec> = genesis_config.into();
    minimal
        .config
        .sequencer_registry
        .sequencer_config
        .is_preferred_sequencer = false;
    let genesis_params = GenesisParams {
        runtime: <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            minimal,
            ValueSetterConfig {
                admin: sov_modules_api::Address::from([0; 28]),
            },
            (),
            PaymasterConfig::default(),
            (),
            (),
        ),
    };

    RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        0,
    )
    .set_config(|conf| {
        conf.sequencer_config = SequencerKindConfig::Preferred(Default::default());
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .start()
    .await
    .unwrap();
}

mod tests_with_basic_kernel {
    use sov_modules_stf_blueprint::GenesisParams;
    use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder};
    use sov_test_utils::{RtAgnosticBlueprint, TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING};

    use super::{
        generate_optimistic_runtime_with_kernel, HighLevelOptimisticGenesisConfig, TestSpec,
    };
    generate_optimistic_runtime_with_kernel!(
        RtWithBasicKernel <=
        kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
        modules: [],
    );

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic(
        expected = "Attempting to use preferred sequencer with an incompatible rollup. Set your sequencer config to `standard` in your rollup's config.toml file or change your kernel to be compatible with soft confirmations."
    )]
    async fn preferred_sequencer_panics_with_basic_kernel() {
        let genesis_config = HighLevelOptimisticGenesisConfig::generate();
        let seq_da_address = HighLevelOptimisticGenesisConfig::<TestSpec>::sequencer_da_addr();
        let genesis_params = GenesisParams {
            runtime: GenesisConfig::from_minimal_config(genesis_config.into()),
        };

        RollupBuilder::<RtAgnosticBlueprint<TestSpec, RtWithBasicKernel<TestSpec>>>::new(
            GenesisSource::CustomParams(genesis_params),
            TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
            0,
        )
        .set_config(|conf| {
            conf.sequencer_config =
                sov_sequencer::SequencerKindConfig::Preferred(Default::default());
        })
        .set_da_config(|c| c.sender_address = seq_da_address)
        .start()
        .await
        .unwrap();
    }
}

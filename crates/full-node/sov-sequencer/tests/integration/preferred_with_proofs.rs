use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::{RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::{
    generate_zk_runtime_with_kernel, test_rollup::TestRollup, RtAgnosticBlueprint, TestSpec,
    TestUser, TEST_DEFAULT_MAX_FEE, TEST_MAX_BATCH_SIZE,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use tokio_stream::StreamExt;

use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, tx_set_value_with_gas};

generate_zk_runtime_with_kernel!(
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    TestRuntime <= value_setter: ValueSetter<S>
);

type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

fn create_genesis_params() -> (GenesisParams<GenesisConfig<TestSpec>>, TestUser<TestSpec>) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
        );

    (
        GenesisParams {
            runtime: rt_genesis_config,
        },
        admin,
    )
}

fn tx_set_value(key: &Ed25519PrivateKey, generation: u64, value_to_set: u64) -> RawTx {
    tx_set_value_with_gas::<TestRuntime<TestSpec>>(
        key,
        generation,
        value_to_set,
        None,
        TEST_DEFAULT_MAX_FEE,
    )
}

async fn create_test_rollup_with_prover() -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let (genesis_params, admin) = create_genesis_params();
    let dir = tempdir_inside_codebase_dir();

    (
        new_test_rollup::<TestRuntime<TestSpec>>(
            dir.clone(),
            genesis_params
                .runtime
                .sequencer_registry
                .sequencer_config
                .seq_da_address,
            genesis_params,
            0,
            true,
            TEST_MAX_BATCH_SIZE,
            BlockProducingConfig::Manual,
            Some(RollupProverConfig::Skip),
            60,
            1000,
            None,
            0,
        )
        .await,
        admin,
    )
}

/// Test proof generation doesn't break the sequencer.
///
/// We run for 50 slots while submitting transactions  and ensure that some aggregated proofs were both produced and processed.
/// (Note that this test uses the mockzkvm, so these proofs are not resource intensive to produce.)
///
/// Aggregate proofs are generated automatically about once every 10 slots, so 50 slots gives us plenty of time to produce a few proofs *and land them on chain*.
/// Any errors should cause a panic in the meantime.
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_proof_generation_doesnt_break_sequencer() -> anyhow::Result<()> {
    let (test_rollup, admin) = create_test_rollup_with_prover().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let mut slot_subscription = client.subscribe_slots().await.unwrap();
    let mut aggregated_proofs = client.subscribe_aggregated_proof().await.unwrap();

    let mut proofs = 0usize;
    let mut tx_generation = 0u64;

    for _ in 0..50 {
        let tx = tx_set_value(&admin.private_key, tx_generation, tx_generation);

        // Retry sending the tx 10 times, with a 3 second delay between attempts.
        for attempt in 0..10 {
            match client.send_raw_tx_to_sequencer(&tx).await {
                Ok(_) => {
                    tx_generation += 1;
                    break;
                }
                Err(e) => {
                    let err_string = e.to_string();
                    let is_503 = err_string.contains("status: 503")
                        || err_string.contains("Service Unavailable");

                    if !is_503 || attempt == 9 {
                        anyhow::bail!(e);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }

        // Produce one block and wait for the corresponding slot notification.
        test_rollup.da_service.produce_block_now().await?;
        let _slot = slot_subscription.next().await.unwrap().unwrap();

        while let Ok(Some(Ok(_proof))) = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            aggregated_proofs.next(),
        )
        .await
        {
            proofs += 1;
        }

        if proofs > 1 {
            break;
        }
    }
    assert!(proofs > 1, "Expected at least 2 proofs, got {proofs}");

    Ok(())
}

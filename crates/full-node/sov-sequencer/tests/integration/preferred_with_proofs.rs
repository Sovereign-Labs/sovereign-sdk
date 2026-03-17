use std::time::Duration;

use anyhow::Context;
use futures::stream::BoxStream;
use serde::Deserialize;
use sov_api_spec::types;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::{RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::{
    generate_zk_runtime_with_kernel, test_rollup::TestRollup, ManualProofPostingControl,
    ManualProofPostingRtAgnosticBlueprint, TestSpec, TestUser, TEST_DEFAULT_MAX_FEE,
    TEST_MAX_BATCH_SIZE, TEST_NORMAL_SHUTDOWN_TIMEOUT,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use tokio::time::{sleep, timeout};
use tokio_stream::StreamExt;

use crate::utils::{
    new_test_rollup_with_manual_proof_posting_and_proof_jump, tempdir_inside_codebase_dir,
    tx_set_value_with_gas, ManualProofPostingTestProverService,
};

generate_zk_runtime_with_kernel!(
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    TestRuntime <= value_setter: ValueSetter<S>
);

type TestBlueprint = ManualProofPostingRtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

const SEQUENCER_RECOVERY_ERROR: &str = "The preferred sequencer is recovering from downtime and cannot provide soft-confirmations at this time";
const AGGREGATED_PROOF_BLOCK_JUMP: usize = 10;
const RECOVERY_DEFERRED_SLOTS_COUNT_OVERRIDE: &str = "20";
const RECOVERY_DRIFT_BLOCKS: usize = 10;
const RETRYABLE_TX_SUBMIT_DELAY: Duration = Duration::from_millis(100);
const PROOF_VISIBILITY_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Deserialize)]
struct ValueResponse {
    value: u32,
}

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

async fn create_test_rollup_with_manual_proof_posting() -> (
    TestRollup<TestBlueprint>,
    ManualProofPostingControl,
    ManualProofPostingTestProverService,
    TestUser<TestSpec>,
) {
    let (genesis_params, admin) = create_genesis_params();
    let dir = tempdir_inside_codebase_dir();

    let (test_rollup, control, prover_service) =
        new_test_rollup_with_manual_proof_posting_and_proof_jump::<TestRuntime<TestSpec>>(
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
            AGGREGATED_PROOF_BLOCK_JUMP,
            0,
        )
        .await;

    (test_rollup, control, prover_service, admin)
}

async fn submit_value_tx(
    client: &sov_api_spec::client::Client,
    key: &Ed25519PrivateKey,
    generation: u64,
    value_to_set: u64,
) -> anyhow::Result<()> {
    let tx = tx_set_value(key, generation, value_to_set);

    for attempt in 0..10 {
        match client.send_raw_tx_to_sequencer(&tx).await {
            Ok(_) => return Ok(()),
            Err(error) => {
                let err_string = error.to_string();
                let is_503 = err_string.contains("status: 503")
                    || err_string.contains("Service Unavailable");

                if !is_503 || attempt == 9 {
                    return Err(error.into());
                }

                sleep(RETRYABLE_TX_SUBMIT_DELAY).await;
            }
        }
    }

    unreachable!("the retry loop should always return before exhausting all branches");
}

async fn produce_block_and_wait_for_sync(
    test_rollup: &TestRollup<TestBlueprint>,
) -> anyhow::Result<()> {
    test_rollup.da_service.produce_block_now().await?;
    test_rollup.wait_for_node_synced().await?;
    Ok(())
}

async fn pause_preferred_batches_and_confirm(
    test_rollup: &TestRollup<TestBlueprint>,
) -> anyhow::Result<()> {
    let mut updates = test_rollup.subscribe_state_updates().await.unwrap();
    test_rollup.pause_preferred_batches().await;
    produce_block_and_wait_for_sync(test_rollup).await?;

    timeout(TestRollup::<TestBlueprint>::POLLING_TIMEOUT, async {
        loop {
            let Some(next) = updates.next().await else {
                anyhow::bail!(
                    "state update subscription closed while waiting for pause acknowledgment"
                );
            };
            let notification = next?;
            if notification.update_skipped_due_to_pause {
                return Ok::<(), anyhow::Error>(());
            }
        }
    })
    .await
    .context("Timed out waiting for preferred batch pause acknowledgment")??;

    Ok(())
}

async fn wait_for_next_proof_ready_to_post(
    test_rollup: &TestRollup<TestBlueprint>,
    control: &ManualProofPostingControl,
    prover_service: &ManualProofPostingTestProverService,
    phase: &str,
) -> anyhow::Result<()> {
    timeout(TestRollup::<TestBlueprint>::POLLING_TIMEOUT, async {
        loop {
            if control.blocks_until_next_aggregate_proof() == 0 {
                prover_service.wait_for_proof_ready_to_post().await;
                return Ok::<(), anyhow::Error>(());
            }

            produce_block_and_wait_for_sync(test_rollup).await?;
        }
    })
    .await
    .with_context(|| {
        format!("Timed out waiting for an aggregate proof to become ready to post during {phase}")
    })??;

    Ok(())
}

async fn wait_until_visible_proof_count(
    test_rollup: &TestRollup<TestBlueprint>,
    aggregated_proofs: &mut BoxStream<'static, anyhow::Result<types::AggregatedProof>>,
    visible_proofs: &mut usize,
    target_visible_proofs: usize,
    phase: &str,
) -> anyhow::Result<()> {
    timeout(TestRollup::<TestBlueprint>::POLLING_TIMEOUT, async {
        while *visible_proofs < target_visible_proofs {
            match timeout(PROOF_VISIBILITY_POLL_INTERVAL, aggregated_proofs.next()).await {
                Ok(Some(Ok(_proof))) => *visible_proofs += 1,
                Ok(Some(Err(error))) => return Err::<(), anyhow::Error>(error),
                Ok(None) => anyhow::bail!("aggregated proof subscription closed unexpectedly"),
                Err(_) => produce_block_and_wait_for_sync(test_rollup).await?,
            }
        }

        Ok::<(), anyhow::Error>(())
    })
    .await
    .with_context(|| {
        format!(
            "Timed out waiting for {target_visible_proofs} aggregate proofs to become visible on the node during {phase}"
        )
    })??;

    Ok(())
}

async fn drive_until_sequencer_ready_state(
    test_rollup: &TestRollup<TestBlueprint>,
    wait_for_ready: bool,
    phase: &str,
) -> anyhow::Result<()> {
    let target_state = if wait_for_ready { "ready" } else { "not-ready" };

    timeout(TestRollup::<TestBlueprint>::POLLING_TIMEOUT, async {
        loop {
            if test_rollup.is_sequencer_ready().await == wait_for_ready {
                return Ok::<(), anyhow::Error>(());
            }

            produce_block_and_wait_for_sync(test_rollup).await?;
        }
    })
    .await
    .with_context(|| {
        format!("Timed out waiting for the sequencer to become {target_state} during {phase}")
    })??;

    Ok(())
}

async fn wait_for_value(
    test_rollup: &TestRollup<TestBlueprint>,
    expected_value: u32,
) -> anyhow::Result<()> {
    timeout(TestRollup::<TestBlueprint>::POLLING_TIMEOUT, async {
        loop {
            let response = test_rollup
                .client
                .query_rest_endpoint::<ValueResponse>("/modules/value-setter/state/value")
                .await;

            match response {
                Ok(response) if response.value == expected_value => {
                    return Ok::<(), anyhow::Error>(())
                }
                Ok(_) | Err(_) => produce_block_and_wait_for_sync(test_rollup).await?,
            }
        }
    })
    .await
    .with_context(|| {
        format!("Timed out waiting for value-setter state to become {expected_value}")
    })??;

    Ok(())
}

/// Verifies that delayed aggregate proofs and an in-progress preferred batch replay correctly
/// across sequencer desync/recovery when proof posting is manually controlled.
///
/// The test releases:
/// - proof 1 while no batch is intentionally held open,
/// - proofs 2 and 3 while a single paused batch stays open and receives interleaved txs,
/// - proof 4 after the sequencer has entered recovery, then opens the gate so later proofs can
///   drain.
///
/// We finish by asserting that five aggregate proofs became visible on the node and that the txs
/// submitted into the paused batch are reflected in state, showing that both delayed proofs and
/// the in-progress batch replayed after the desync.
#[tokio::test(flavor = "multi_thread")]
async fn test_manual_proof_posting_recovery_replays_batch_and_proofs() -> anyhow::Result<()> {
    let (test_rollup, control, prover_service, admin) =
        create_test_rollup_with_manual_proof_posting().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup
        .wait_for_sequencer_ready()
        .await
        .context("sequencer did not become ready during startup")?;

    let client = test_rollup.api_client().clone();
    let mut aggregated_proofs = client.subscribe_aggregated_proof().await.unwrap();
    let mut visible_proofs = 0usize;

    submit_value_tx(&client, &admin.private_key, 0, 10).await?;
    wait_for_next_proof_ready_to_post(&test_rollup, &control, &prover_service, "proof 1").await?;
    control.release_next_proof();
    wait_until_visible_proof_count(
        &test_rollup,
        &mut aggregated_proofs,
        &mut visible_proofs,
        1,
        "proof 1",
    )
    .await?;

    pause_preferred_batches_and_confirm(&test_rollup).await?;

    submit_value_tx(&client, &admin.private_key, 1, 20).await?;
    wait_for_next_proof_ready_to_post(&test_rollup, &control, &prover_service, "proof 2").await?;
    control.release_next_proof();

    submit_value_tx(&client, &admin.private_key, 2, 30).await?;
    wait_for_next_proof_ready_to_post(&test_rollup, &control, &prover_service, "proof 3").await?;
    control.release_next_proof();

    submit_value_tx(&client, &admin.private_key, 3, 40).await?;
    wait_for_next_proof_ready_to_post(&test_rollup, &control, &prover_service, "proof 4").await?;

    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT",
        RECOVERY_DEFERRED_SLOTS_COUNT_OVERRIDE,
    );
    for _ in 0..RECOVERY_DRIFT_BLOCKS {
        test_rollup.da_service.produce_block_now().await?;
    }
    test_rollup.wait_for_node_synced().await?;

    test_rollup.resume_preferred_batches().await;
    drive_until_sequencer_ready_state(&test_rollup, false, "recovery trigger").await?;
    test_rollup.wait_for_node_synced().await?;
    test_rollup.wait_for_sequencer_not_ready().await?;

    let recovery_tx = tx_set_value(&admin.private_key, 4, 50);
    let err = client
        .send_raw_tx_to_sequencer(&recovery_tx)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(SEQUENCER_RECOVERY_ERROR),
        "Expected recovery error, got: {err}",
    );

    control.release_next_proof();
    control.open();

    wait_until_visible_proof_count(
        &test_rollup,
        &mut aggregated_proofs,
        &mut visible_proofs,
        5,
        "proof replay after desync",
    )
    .await?;

    assert_eq!(visible_proofs, 5, "Expected exactly 5 visible proofs");

    // These txs all use sequential generations from the same account, so reaching the final value
    // proves the paused in-progress batch replayed intact across recovery.
    wait_for_value(&test_rollup, 40).await?;

    control.open();
    std::env::remove_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT");
    timeout(TEST_NORMAL_SHUTDOWN_TIMEOUT, test_rollup.shutdown())
        .await
        .context("Timed out shutting down test rollup")??;

    Ok(())
}

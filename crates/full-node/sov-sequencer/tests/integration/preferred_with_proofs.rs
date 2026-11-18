use sov_modules_api::{DispatchCall, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::{
    generate_zk_runtime_with_kernel,
    test_rollup::{next_within, TestRollup},
    RtAgnosticBlueprint, TestSpec, TestUser,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use tokio_stream::StreamExt;

use crate::utils::{tempdir_inside_codebase_dir, SeqTestRollup};

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

fn set_value(value_to_set: u64) -> <TestRuntime<TestSpec> as DispatchCall>::Decodable {
    <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetValue {
            value: value_to_set as u32,
            gas: None,
        },
    )
}

async fn create_test_rollup_with_prover() -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let (genesis_params, admin) = create_genesis_params();
    let dir = tempdir_inside_codebase_dir();

    (
        SeqTestRollup::<TestRuntime<TestSpec>>::new(
            genesis_params
                .runtime
                .sequencer_registry
                .sequencer_config
                .seq_da_address,
            genesis_params,
        )
        .dir(dir.clone())
        .rollup_prover_config(RollupProverConfig::Prove)
        .max_batch_execution_time_millis(1000)
        .finalization_blocks(0)
        .start()
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
async fn test_proof_generation_doesnt_break_sequencer() -> anyhow::Result<()> {
    let (test_rollup, admin) = create_test_rollup_with_prover().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();
    let mut slot_subscription = client.subscribe_slots().await.unwrap();
    let mut aggregated_proofs = client.subscribe_aggregated_proof().await.unwrap();

    let mut proofs = 0usize;
    let mut accepted_txs = 0u64;

    for _ in 0..50 {
        // Retry sending the tx 10 times, with a 3 second delay between attempts.
        for attempt in 0..10 {
            match test_rollup.submit(&admin, set_value(accepted_txs)).await {
                Ok(_) => {
                    accepted_txs += 1;
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
        let _slot = next_within(
            &mut slot_subscription,
            TestRollup::<TestBlueprint>::POLLING_TIMEOUT,
        )
        .await
        .unwrap();

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
